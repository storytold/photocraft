//! Move tool › Show Highlight on Rollover (Preferences › Tools, on by default): with Auto-Select
//! on, or ⌘ held while it is off, hovering the canvas outlines the layer (or group, per the
//! Auto-Select target) that a click there would pick. The hit test is the click's own
//! (`pick_cmds::auto_select_target`), so a type layer is outlined anywhere inside its text, as it
//! is picked there (#2591). The answer is cached per document revision and pointer pixel: a
//! pointer resting on the canvas costs nothing per frame.

use egui::{Pos2, Stroke};
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

/// The last answer: what a click at `at` on that revision of the document would outline.
pub(crate) struct Hover {
    doc: DocId,
    revision: u64,
    at: [i32; 2],
    group: bool,
    bounds: Option<Rect>,
}

/// The document-space bounds to outline with the pointer at `at` (document px), or `None` when
/// nothing is highlighted: the preference is off, another tool is current, a click wouldn't
/// Auto-Select (it's off without ⌘, or the selected pixels move instead), a button is down, or a
/// drag or transform is under way.
pub(crate) fn target(app: &mut PhotocraftApp, at: Option<[f64; 2]>, command: bool, pressed: bool) -> Option<Rect> {
    if !app.session.prefs().tools.show_highlight_on_rollover
        || app.ui.tool != Tool::Move
        || pressed
        || app.drag.is_some()
        || app.guide_drag.is_some()
        || app.ui.transform.is_some()
        || app.ui.tool_options.move_auto_select == command
        || crate::move_ui::moves_selected_pixels(app)
    {
        return None;
    }
    let at = at.filter(|p| p[0].is_finite() && p[1].is_finite())?;
    let at = [at[0].floor() as i32, at[1].floor() as i32];
    let group = app.ui.tool_options.move_target == "group";
    let st = app.session.active()?;
    let (doc, revision) = (st.doc.id, st.revision);
    if let Some(h) = app.move_hover.as_ref().filter(|h| h.doc == doc && h.revision == revision && h.at == at && h.group == group) {
        return h.bounds;
    }
    let bounds = photocraft_engine::pick_cmds::auto_select_target(&st.doc, at[0], at[1], group).and_then(|id| outline(&st.doc, id));
    app.move_hover = Some(Hover { doc, revision, at, group, bounds });
    bounds
}

/// The outline of a picked layer: its visible content (a group's, its visible layers' together).
/// Not the Background: it fills the canvas, and the Move tool doesn't move it.
fn outline(doc: &Document, id: LayerId) -> Option<Rect> {
    let rows = doc.walk();
    let (prefix, _, top) = rows.iter().find(|(_, _, l)| l.id == id)?;
    if crate::doc_props_ui::is_background(doc, top) {
        return None;
    }
    let b = rows
        .iter()
        .filter(|(p, _, _)| p.starts_with(prefix) && (prefix.len()..=p.len()).all(|n| p.get(..n).and_then(|q| doc.layer_at(q)).is_some_and(|l| l.visible)))
        .fold(Rect::EMPTY, |acc, (_, _, l)| acc.union(&bounds(doc, l)));
    (!b.is_empty()).then_some(b)
}

fn bounds(doc: &Document, l: &Layer) -> Rect {
    if let Some(a) = l.artboard() {
        return a.rect;
    }
    let content = |s| photocraft_compose::bounds::content_bounds(s);
    match &l.content {
        LayerContent::Group(_) | LayerContent::Adjustment(_) => Rect::EMPTY,
        // A fill layer covers the canvas; its mask limits it.
        LayerContent::Fill(_) => l.mask.as_ref().filter(|m| m.enabled).map_or(doc.bounds(), |m| content(&m.surface)),
        _ => l.surface().map_or(Rect::EMPTY, content),
    }
}

/// Draws the highlight on the active document's canvas, in the theme's accent colour.
pub(crate) fn draw(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, idx: usize, hover: Option<Pos2>) {
    if app.session.active_index() != Some(idx) {
        return;
    }
    let (mods, pressed) = painter.ctx().input(|i| (i.modifiers, i.pointer.any_down()));
    let command = crate::workspace_ui::sticky_mods(app, mods).command;
    let at = hover.map(|p| xf.to_doc(p));
    let Some(b) = target(app, at, command, pressed) else { return };
    let accent = crate::theme::Tokens::get(painter.ctx()).accent;
    painter.rect_stroke(xf.doc_rect(b), 0.0, Stroke::new(1.5, accent), egui::StrokeKind::Outside);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rollover_outlines_the_layer_a_click_would_pick() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, a| {
                doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(8, 8, 24, 24), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.ui.tool = Tool::Move;
        let a = Rect::new(8, 8, 24, 24);
        // The empty top layer is clicked through to the painted one; the Background isn't outlined.
        assert_eq!(target(&mut app, Some([10.5, 12.0]), false, false), Some(a));
        assert_eq!(target(&mut app, Some([40.0, 40.0]), false, false), None);
        // Nothing while a button is down or a drag runs.
        assert_eq!(target(&mut app, Some([10.5, 12.0]), false, true), None);
        // Auto-Select off: only with ⌘ held.
        app.ui.tool_options.move_auto_select = false;
        assert_eq!(target(&mut app, Some([10.5, 12.0]), false, false), None);
        assert_eq!(target(&mut app, Some([10.5, 12.0]), true, false), Some(a));
        // Preferences › Tools › Show Highlight on Rollover off.
        app.session.edit_prefs(|p| p.tools.show_highlight_on_rollover = false);
        assert_eq!(target(&mut app, Some([10.5, 12.0]), true, false), None);
    }
}
