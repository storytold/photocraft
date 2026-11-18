//! Move tool › Show Highlight on Rollover (Preferences › Tools, on by default): with Auto-Select
//! on, or ⌘ held while it is off, hovering the canvas outlines the layer (or group, per the
//! Auto-Select target) that a click there would pick. The hit test is the click's own
//! (`pick_cmds::move_auto_select_target`), so a type layer is outlined anywhere inside its text, as it
//! is picked there (#2591). The answer is cached per document revision and pointer pixel: a
//! pointer resting on the canvas costs nothing per frame.

use craft::behavior::kurbo;
use craft::ui::canvas_controls::{HoverOutline, HoverShape};
use craft::ui::contract::Show as _;
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The last answer: what a click at `at` on that revision of the document would outline.
pub(crate) struct Hover {
    doc: DocId,
    revision: u64,
    at: [i32; 2],
    group: bool,
    target_layer: Option<LayerId>,
    bounds: Option<Rect>,
}

/// The document-space bounds to outline with the pointer at `at` (document px), or `None` when
/// nothing is highlighted: the preference is off, another tool is current, a click wouldn't
/// Auto-Select (it's off without ⌘, or the selected pixels move instead), a button is down (which
/// covers every drag), or a transform is under way.
pub(crate) fn target(app: &mut PhotocraftApp, at: Option<[f64; 2]>, command: bool, pressed: bool) -> Option<Rect> {
    if !app.session.prefs().tools.show_highlight_on_rollover
        || app.ui.tool != Tool::Move
        || !app.ui.tool_options.move_show_hover_bounds
        || pressed
        || app.ui.transform.is_some()
        || app.ui.tool_options.move_auto_select == command
        || crate::move_ui::moves_selected_pixels_with(app, Tool::Move)
    {
        app.canvas_hover_layer = None;
        return None;
    }
    let Some(at) = at.filter(|p| p[0].is_finite() && p[1].is_finite()) else {
        app.canvas_hover_layer = None;
        return None;
    };
    let at = [at[0].floor() as i32, at[1].floor() as i32];
    let group = app.ui.tool_options.move_target == "group";
    let st = app.session.active()?;
    let (doc, revision) = (st.doc.id, st.revision);
    if let Some(h) = app.move_hover.as_ref().filter(|h| h.doc == doc && h.revision == revision && h.at == at && h.group == group) {
        app.canvas_hover_layer = h.target_layer;
        return h.bounds;
    }
    let target_layer = photocraft_engine::pick_cmds::move_auto_select_target(&st.doc, at[0], at[1], group);
    let bounds = target_layer.and_then(|id| outline(&st.doc, id));
    let target_layer = if bounds.is_some() { target_layer } else { None };
    app.move_hover = Some(Hover { doc, revision, at, group, target_layer, bounds });
    app.canvas_hover_layer = target_layer;
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

/// The document-space bounds to outline: either a layer hovered in the Layers panel, or what a
/// click at `at` would pick with the Move tool.
pub(crate) fn active_outline(app: &mut PhotocraftApp, at: Option<[f64; 2]>, command: bool, pressed: bool) -> Option<Rect> {
    if !app.session.prefs().tools.show_highlight_on_rollover {
        app.canvas_hover_layer = None;
        return None;
    }
    // A layer hovered in the Layers panel takes precedence over canvas rollover.
    if let Some(id) = app.hover_layer
        && (app.ui.tool != Tool::Move || app.ui.tool_options.move_show_layers_hover_bounds)
        && !pressed
        && app.ui.transform.is_none()
        && let Some(st) = app.session.active()
    {
        return outline(&st.doc, id);
    }
    target(app, at, command, pressed)
}

/// Shows the highlight over the active document's canvas `canvas`; `pointer` is the screen hover
/// position. The Layers panel reads `canvas_hover_layer`, so a change of it asks for a repaint.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, canvas: egui::Rect, pointer: Option<egui::Pos2>) {
    let prev_canvas_hover = app.canvas_hover_layer;
    let command = craft::ui::runtime::pointer::frame(ui.ctx()).mods.cmd;
    let pressed = crate::canvas::tools::in_gesture(app);
    let at = pointer.and_then(|p| crate::canvas::document_display::doc_point_at(app, p)).map(|p| [p.x, p.y]);
    let outline = active_outline(app, at, command, pressed);
    if app.canvas_hover_layer != prev_canvas_hover {
        ui.ctx().request_repaint();
    }
    let shape = outline.map(|b| HoverShape::Rect(kurbo::Rect::new(f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1))));
    HoverOutline::new(tl!("Layer rollover"), shape).show_at(ui, canvas);
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
        app.session
            .edit("paint top", |doc, active| {
                let top = doc.layer_mut(active.unwrap()).unwrap();
                top.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 32), &[0.0, 0.0, 1.0, 1.0]);
                top.locks.position = true;
                Ok(())
            })
            .unwrap();
        assert_eq!(target(&mut app, Some([10.5, 12.0]), false, false), Some(a), "the rollover skips the position-locked top layer");
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

    #[test]
    fn layer_panel_hover_outlines_layer_on_canvas() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, a| {
                doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(8, 8, 24, 24), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        let target_layer = app.session.active().unwrap().active_layer.unwrap();
        let a = Rect::new(8, 8, 24, 24);

        // Even with a different tool (e.g. Brush) and no canvas hover, hovering the layer in the panel outlines it:
        app.ui.tool = Tool::Brush;
        assert_eq!(active_outline(&mut app, None, false, false), None);
        app.hover_layer = Some(target_layer);
        assert_eq!(active_outline(&mut app, None, false, false), Some(a));

        // While a button is down (a drag), no outline:
        assert_eq!(active_outline(&mut app, None, false, true), None);

        // When rollover preference is disabled, no outline:
        app.session.edit_prefs(|p| p.tools.show_highlight_on_rollover = false);
        assert_eq!(active_outline(&mut app, None, false, false), None);
    }

    #[test]
    fn canvas_hover_sets_canvas_hover_layer() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, a| {
                doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(8, 8, 24, 24), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        let target_layer = app.session.active().unwrap().active_layer.unwrap();
        app.ui.tool = Tool::Move;

        // Hovering over painted layer sets canvas_hover_layer:
        assert!(target(&mut app, Some([10.5, 12.0]), false, false).is_some());
        assert_eq!(app.canvas_hover_layer, Some(target_layer));

        // Hovering empty area clears canvas_hover_layer:
        assert!(target(&mut app, Some([40.0, 40.0]), false, false).is_none());
        assert_eq!(app.canvas_hover_layer, None);
    }
}
