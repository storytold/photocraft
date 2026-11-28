//! The Properties panel: an `Inspector` over the active layer (or the document), whose body is the
//! controls of what is targeted: a mask, the document, an adjustment, an artboard, a frame or a layer.

use craft::behavior::IconId;
use craft::ui::buttons::Button;
use craft::ui::choice::Switch;
use craft::ui::contract::{Bind, Props as _, Show as _, Sizing, clicked};
use craft::ui::patterns::panels::Inspector;
use craft::ui::primitives::IconSpec;

use photocraft_doc::{Layer, LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

pub(super) mod adjustments;

/// What the panel's body shows, from what is targeted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Body {
    /// The active layer's pixel mask.
    Mask,
    /// The active layer's vector mask.
    VectorMask,
    /// The document: nothing, or the Background layer, is active.
    Document,
    /// The active layer.
    Layer,
}

/// Icon for a layer kind (the same glyphs as the Layers panel's kind filter).
fn kind_icon(content: &LayerContent) -> &'static str {
    match content {
        LayerContent::Adjustment(_) => "adjustments",
        LayerContent::Group(_) => "folder",
        LayerContent::Fill(_) => "paint-bucket",
        LayerContent::Text(_) => "type",
        LayerContent::Shape(_) => "vector-shape",
        LayerContent::Smart(_) => "app-window",
        LayerContent::Raster(_) => "image",
    }
}

/// "Type Layer", "Curves", "Artboard"…
fn kind_label(layer: &Layer) -> String {
    if layer.frame.is_some() {
        return tl!("Frame").into();
    }
    match &layer.content {
        LayerContent::Adjustment(a) => a.label().to_string(),
        LayerContent::Group(g) if g.artboard.is_some() => "Artboard".to_string(),
        other => format!("{} Layer", other.kind_name()),
    }
}

/// The Properties panel body: the document and layer properties, mask properties when a mask
/// thumbnail is targeted, and an adjustment layer's controls. The target (document, active layer,
/// mask targets, the layer the panel was opened for) keys the inspector, so a field's typing or
/// scrub state stays with its layer.
pub(super) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // A run that shows no adjustment layer's controls ends the Reset session kept (`PhotocraftApp::properties_hidden`).
    // What the run shows decides, not whether the controls' code ran: a pass that reuses the inspector's measurement
    // runs none of its content.
    if !body(app, ui) {
        app.properties_hidden = app.properties_hidden.saturating_add(2);
    }
}

/// The panel's inspector; whether it shows an adjustment layer's controls.
fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    let inspector = Inspector::new(tl!("Properties")).empty(tl!("No properties"));
    let Some(st) = app.session.active() else {
        inspector.show(ui, |_, _| {});
        return false;
    };
    let doc = st.doc.clone();
    let active = st.active_layer;
    let selected = st.selected_layers().len();
    let target = (doc.id, active, app.ui.mask_target, app.ui.vector_mask_target, app.ui.properties_layer);
    let layer = active.and_then(|id| doc.layer(id));
    // Photoshop shows the Document properties when nothing or the Background layer is selected,
    // with Align and Distribute above them when the Background is one of several (#4608).
    let shown = match layer {
        Some(l) if crate::mask_props_ui::targeted(app, l) => Body::Mask,
        Some(l) if crate::mask_props_ui::vector_targeted(app, l) => Body::VectorMask,
        _ if crate::doc_props_ui::shows_document(&doc, active) => Body::Document,
        _ => Body::Layer,
    };
    let adjustment = shown == Body::Layer && layer.is_some_and(|l| matches!(l.content, LayerContent::Adjustment(_)));
    let mut inspector = inspector.target(target);
    // The Document properties are headed by the document, as production's panel is.
    inspector = match layer {
        Some(l) if shown != Body::Document => {
            inspector.header(l.name.clone()).icon(IconSpec::svg(IconId::new(kind_icon(&l.content)))).kind(tl!(&kind_label(l)).to_string())
        }
        _ => inspector.header(tl!("Document")).icon(IconSpec::svg(IconId::new("file"))),
    };
    inspector.show(ui, |ui, _| {
        let Some(layer) = layer else {
            if shown == Body::Document {
                document_body(app, ui, selected);
            }
            return;
        };
        match shown {
            Body::Mask if layer.frame.is_some() => crate::frame_tool::properties(app, ui, layer),
            Body::Mask => crate::mask_props_ui::properties(app, ui, layer),
            Body::VectorMask => crate::mask_props_ui::vector_properties(app, ui, layer),
            Body::Document => document_body(app, ui, selected),
            Body::Layer => layer_body(app, ui, layer),
        }
    });
    adjustment
}

/// The document's properties, with Align and Distribute above them while several layers are selected.
fn document_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, selected: usize) {
    if selected > 1 {
        crate::layer_props_ui::selection_align(ui);
    }
    crate::doc_props_ui::properties(app, ui);
}

/// The controls of the active layer by kind. A frame comes before the kind sections.
fn layer_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    if let LayerContent::Adjustment(adj) = &layer.content {
        adjustment_controls(app, ui, layer.id, adj);
    } else if layer.artboard().is_some() {
        crate::artboard_ui::properties(app, ui, layer);
    } else if layer.frame.is_some() {
        crate::frame_tool::properties(app, ui, layer);
    } else {
        // Transform, Align, the kind's sections and Quick Actions.
        crate::layer_props_ui::properties(app, ui, layer);
    }
}

/// An adjustment layer's controls: its scope, its editor, its visibility and Reset. The scope comes
/// first so it stays reachable before tall editors (#5231); adjustment layers affect the stack by
/// default and clipping limits them to their base.
fn adjustment_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: LayerId, adj: &photocraft_doc::Adjustment) {
    let layer = |app: &PhotocraftApp| app.session.active().and_then(|s| s.doc.layer(id).map(|l| (l.clipped, l.visible)));
    if let Some((clipped, _)) = layer(app)
        && let Some(&on) = Switch::new(Bind::given(clipped)).label(tl!("Clip to layer below")).show(ui).events.committed()
    {
        let _ = app.run("layer.setProps", json!({"layer": id.0, "clipped": on}));
    }
    ui.add_space(6.0);
    crate::adjust_editors::layer_editor(app, ui, id, adj);
    ui.add_space(6.0);
    if let Some((_, visible)) = layer(app)
        && let Some(&on) = Switch::new(Bind::given(visible)).label(tl!("Adjustment visible")).show(ui).events.committed()
    {
        let _ = app.run("layer.setProps", json!({"layer": id.0, "visible": on}));
    }
    ui.add_space(6.0);
    if clicked(&Button::new(tl!("Reset to defaults")).sizing(Sizing::FillWidth).show(ui)) {
        // Photoshop's two-step Reset where measured (Color Balance), else the defaults.
        let params = crate::adjust_editors::layer_reset(app, id, adj);
        let _ = app.run("layer.setAdjustment", params);
        app.live_adjust = None;
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use photocraft_doc::LayerId;

    use super::*;

    /// #4608: with the Background one of several selected layers (and the active one), Properties
    /// shows the Document properties as before, with Align and Distribute above them, and its buttons
    /// align the selection: the Background stays, the other layer lines up with it.
    #[test]
    fn background_in_a_multiple_selection_keeps_align_and_distribute() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 300, "height": 200})).unwrap();
        let bg = s.active().unwrap().doc.layers[0].id.0;
        let block = s.execute("layer.new.layer", json!({"name": "Block"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("select.rect", json!({"x": 20, "y": 30, "width": 40, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        // Background alone: the Document properties only.
        s.execute("layer.select", json!({"layer": bg})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        // The Move tool's options bar has its own Align buttons: a tool without them leaves the Properties panel's the only ones.
        app.ui.tool = crate::state::Tool::Brush;
        let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |_cc| app);
        crate::panel_docking::show_panel(h.state_mut(), "properties", Some(true)).unwrap();
        h.run_steps(8);
        assert!(h.query_by_label("Align Horizontal Centers").is_none(), "one layer: Document properties only");
        // The Background is still the active layer once Block joins the selection.
        h.state_mut().run("layer.select", json!({"layer": block, "mode": "add"})).unwrap();
        h.state_mut().run("layer.select", json!({"layer": bg, "mode": "add"})).unwrap();
        h.run_steps(8);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.active_layer, Some(LayerId(bg)));
        assert_eq!(st.selected_layers().len(), 2);
        h.get_by_label("Align Horizontal Centers").click();
        h.run_steps(4);
        let st = h.state().session.active().unwrap();
        let b = st.doc.layer(LayerId(block)).unwrap().surface().unwrap().content_bounds();
        assert_eq!((b.x0 + b.x1) / 2, 150, "centred on the Background, which spans the canvas: {b:?}");
        assert_eq!(b.y0, 30, "only horizontally");
    }
}
