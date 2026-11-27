//! #10884: Select › Find Layers (⌥⇧⌘F) filters the Layers panel by Name, as in Photoshop: it shows
//! the Layers panel, switches the filter to Name and puts the caret in its field with the text
//! selected, so typing replaces it. The layer selection is not changed. With a `name` param
//! (scripts and agents) it sets the Name text to it instead of focusing the field.

use craft::layers::ui::filter::FilterBy;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Run `select.findLayers`: the engine checks `params` and lists the layers the Name text matches;
/// then the Name filter is applied. Without a `name` the text is kept, and the Layers panel is
/// shown with the field focused.
pub fn invoke(app: &mut PhotocraftApp, params: Value) -> Result<Value, String> {
    let given = params.get("name").is_some_and(|n| !n.is_null());
    // `run` keeps the Name text when no `name` is given ([`params_for_run`]) and applies the
    // reply's Name filter ([`apply`]).
    let reply = app.run("select.findLayers", params)?;
    if !given {
        crate::panel_docking::show_panel(app, "layers", Some(true))?;
        // A hidden filter row (Filter Options › Hide) stays hidden, as in Photoshop, and showing it
        // later doesn't put the caret in the field: only a drawn row takes the focus.
        app.find_layers_focus = app.ui.layers_settings.show_filters;
    }
    Ok(reply)
}

/// The params `select.findLayers` runs with: without a `name` the panel's Name text is kept, so the
/// reply lists the layers the panel lists however the command runs (the menu, the shortcut,
/// `engine.execute` or an action step).
pub(crate) fn params_for_run(app: &PhotocraftApp, params: Value) -> Value {
    if params.get("name").is_some_and(|n| !n.is_null()) {
        return params;
    }
    let mut params = if params.is_object() { params } else { json!({}) };
    params["name"] = json!(app.ui.layer_filter.name);
    params
}

/// Apply the Name filter in a `select.findLayers` reply to the Layers panel: switch to Name and set
/// the text when the reply has one, which deselects the layers it hides. [`PhotocraftApp::run`] calls it after every Find Layers.
pub(crate) fn apply(app: &mut PhotocraftApp, reply: &Value) {
    let mut filter = app.ui.layer_filter.clone();
    filter.by = FilterBy::Name;
    if let Some(name) = reply["layerFilter"]["name"].as_str() {
        filter.name = name.to_string();
    }
    super::apply(app, filter);
}
