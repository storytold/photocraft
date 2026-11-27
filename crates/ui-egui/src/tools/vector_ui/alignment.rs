//! The one Align Edges checkbox of the shape tools, the Pen in Shape mode and a shape layer's
//! Properties, as in the reference application. In the options bar it is the tool option
//! (`ToolOptions::align_edges`): a drag lands on whole pixels and the new shape layer gets Align
//! Edges. With a shape layer selected it shows and edits that layer's flag
//! (`shape.edit {useAlignedRendering}`) too, as Fill and Stroke do. The flag only changes how the
//! layer renders (`photocraft_vector::align_edges`); its path is never moved.
use crate::PhotocraftApp;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _};
use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

/// The checkbox for `target` (a shape layer: shows and edits its flag) and, with `defaults`, the
/// tool option the next shape takes. Greyed when not `enabled` or when the layer is locked.
pub(crate) fn option(app: &mut PhotocraftApp, ui: &mut egui::Ui, target: Option<LayerId>, defaults: bool, enabled: bool) {
    let layer = target.and_then(|id| {
        app.session.active().and_then(|st| {
            let LayerContent::Shape(sh) = &st.doc.layer(id)?.content else { return None };
            let locks = st.doc.effective_locks(id);
            Some((sh.use_aligned_rendering, locks.all || locks.pixels))
        })
    });
    let before = layer.map_or(app.ui.tool_options.align_edges, |v| v.0);
    let mut value = before;
    Checkbox::new(Bind::bound(&mut value)).label(tl!("Align Edges").to_owned()).enabled(enabled && !layer.is_some_and(|v| v.1)).show(ui);
    if value == before {
        return;
    }
    if let Some(id) = target.filter(|_| layer.is_some())
        && let Err(e) = app.run("shape.edit", json!({"layer": id.0, "useAlignedRendering": value}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
        return;
    }
    if defaults {
        app.ui.tool_options.align_edges = value;
    }
}

/// The shape tools' checkbox, at the options bar's right end: enabled in Shape mode only, where it
/// also edits the selected shape layer.
pub(super) fn shape_tool_option(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let shape_mode = app.ui.tool_options.shape_mode == super::VectorMode::Shape;
    let target = if shape_mode { super::options::selected_shape(&app.session) } else { None };
    option(app, ui, target, true, shape_mode);
}

/// The Pen's checkbox: enabled in Shape mode only. While a path is being drawn it is the value the
/// draft captured (the layer it joins, or the tool option it started with).
pub(super) fn pen_option(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let shape_mode = app.ui.tool_options.vector_mode == super::VectorMode::Shape;
    let draft = app.ui.pen.as_ref().and_then(|p| p.owner.as_ref()).filter(|o| shape_mode && o.valid(app)).map(|o| o.options.align_edges);
    if let Some(before) = draft {
        let mut value = before;
        Checkbox::new(Bind::bound(&mut value)).label(tl!("Align Edges").to_owned()).show(ui);
        if value != before {
            app.ui.tool_options.align_edges = value;
            if let Some(owner) = app.ui.pen.as_mut().and_then(|p| p.owner.as_mut()) {
                owner.options.align_edges = value;
            }
        }
    } else {
        let target = if shape_mode && app.ui.pen.is_none() && super::pen::pen_path_op(&app.ui.tool_options).is_some() {
            super::options::selected_shape(&app.session)
        } else {
            None
        };
        option(app, ui, target, true, shape_mode);
    }
}
