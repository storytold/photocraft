//! #10884: Select › Find Layers (⌥⇧⌘F) filters the Layers panel by Name, as in Photoshop: it shows
//! the Layers panel, switches the filter to Name and puts the caret in its field with the text
//! selected, so typing replaces it. The layer selection is not changed. With a `name` param
//! (scripts and agents) it sets the Name text to it instead of focusing the field.

use serde_json::{Value, json};

use super::FilterBy;
use crate::{PhotocraftApp, widgets};

/// The egui memory flag: focus the Name field the next time it is drawn.
fn focus_flag() -> egui::Id {
    egui::Id::unique("layer-filter-find-focus")
}

/// Run `select.findLayers`: the engine checks `params` and lists the layers the Name text matches;
/// then the Name filter is applied. Without a `name` the text is kept, and the Layers panel is
/// shown with the field focused.
pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Result<Value, String> {
    let given = params.get("name").is_some_and(|n| !n.is_null());
    let params = if given { params } else { json!({"name": app.ui.layer_filter_name}) };
    let reply = app.run("select.findLayers", params)?;
    app.ui.layer_filter_by = FilterBy::Name;
    if let Some(name) = reply["layerFilter"]["name"].as_str() {
        app.ui.layer_filter_name = name.to_string();
    }
    if !given {
        *crate::dock::Group::Layers.tab_mut(&mut app.ui.dock_tabs) = 0;
        crate::dock::reveal(app, crate::dock::Group::Layers);
        ctx.data_mut(|d| d.insert_temp(focus_flag(), true));
    }
    Ok(reply)
}

/// The Name filter's text field, taking the focus with its text selected after [`invoke`].
pub fn name_field(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let id = ui.scope_id().with("layer-filter-name");
    if ui.ctx().data_mut(|d| d.remove_temp::<bool>(focus_flag())).unwrap_or(false) {
        ui.memory_mut(|m| m.request_focus(id));
        widgets::field_tab::select_all(ui.ctx(), id, &app.ui.layer_filter_name);
    }
    let width = ui.available_width().max(48.0);
    widgets::text_field(ui, id, &mut app.ui.layer_filter_name, tl!("Search"), width, false);
}
