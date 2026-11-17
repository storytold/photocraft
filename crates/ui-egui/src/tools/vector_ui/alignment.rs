//! App policy for the shared framework checkbox.
//! ShapeBar/ShapePanel do not expose the engine-specific useAlignedRendering field;
//! compose the framework Checkbox here and keep document edits in engine commands.
use crate::PhotocraftApp;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _};
use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;
pub(crate) fn option(app: &mut PhotocraftApp, ui: &mut egui::Ui, target: Option<LayerId>, defaults: bool) {
    let layer = target.and_then(|id| {
        app.session.active().and_then(|st| {
            let LayerContent::Shape(sh) = &st.doc.layer(id)?.content else { return None };
            let locks = st.doc.effective_locks(id);
            Some((sh.use_aligned_rendering, locks.all || locks.pixels))
        })
    });
    let before = layer.map_or(app.ui.tool_options.use_aligned_rendering, |v| v.0);
    let mut value = before;
    Checkbox::new(Bind::bound(&mut value)).label(tl!("Align Edges").to_owned()).enabled(!layer.is_some_and(|v| v.1)).show(ui);
    if value == before {
        return;
    }
    if let Some(id) = target
        && let Err(e) = app.run("shape.edit", json!({"layer": id.0, "useAlignedRendering": value}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
        return;
    }
    if defaults {
        app.ui.tool_options.use_aligned_rendering = value;
    }
}
pub(super) fn pen_option(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let draft = app.ui.pen.as_ref().and_then(|p| p.owner.as_ref()).filter(|o| o.valid(app)).map(|o| o.options.use_aligned_rendering);
    if let Some(before) = draft {
        let mut value = before;
        Checkbox::new(Bind::bound(&mut value)).label(tl!("Align Edges").to_owned()).show(ui);
        if value != before {
            app.ui.tool_options.use_aligned_rendering = value;
            if let Some(owner) = app.ui.pen.as_mut().and_then(|p| p.owner.as_mut()) {
                owner.options.use_aligned_rendering = value;
            }
        }
    } else {
        let target =
            if app.ui.pen.is_none() && super::pen::pen_path_op(&app.ui.tool_options).is_some() { super::options::selected_shape(&app.session) } else { None };
        option(app, ui, target, true);
    }
}
