//! Type panels: the Style Options editor of the Character Styles and Paragraph Styles panels, the colour picker
//! of a style's text colour, and the shared view state ([`TypePanelsUi`]).
//!
//! Everything acts through engine commands (`type.characterStyle.*`, `type.paragraphStyle.*`,
//! `type.insertText`, `edit.checkSpelling`); the panels only hold view state, part of the serialisable
//! UI state so the control channel can drive it. The panel bodies live in `styles`, `glyphs` and `spelling`.

use craft::behavior::Selection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;

pub(crate) mod color_picker;
pub(crate) mod glyphs;
pub(crate) mod spelling;
pub(crate) mod style_options;
pub(crate) mod styles;

#[cfg(test)]
pub(crate) use spelling::close_spelling;
pub use spelling::{SPELLING, SpellDialog, open_spelling};

/// View state of the type panels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypePanelsUi {
    /// Style Options editor open for (paragraph?, id).
    pub options: Option<(bool, u32)>,
    /// The selected row of the Character Styles list (0) and of the Paragraph Styles list (1).
    #[serde(skip)]
    pub style_selection: [Selection<u64>; 2],
    /// The Glyphs panel's own family and style; `None` follows the type selection.
    pub glyph_face: Option<(String, String)>,
    /// The Glyphs panel's category, zoom and recent glyphs. Saved with the UI state because PhotoCraft's runtime
    /// has no craft-host `Host` to hold a setting.
    pub glyph_settings: craft::font_manager::glyphs::GlyphsPanelSettings,
    /// The text style colour picker's target while it is open.
    #[serde(skip)]
    pub(crate) color_target: Option<color_picker::Target>,
    /// Edit › Check Spelling dialog.
    pub spell: Option<spelling::SpellDialog>,
    /// Check All Layers as the dialog was last closed in this session; Edit › Check Spelling… opens
    /// with it when no `allLayers` is given (#14183).
    pub spell_all_layers: Option<bool>,
}

/// Edit › Check Spelling… (opens the dialog when run without an `action`), and Character/Paragraph Styles with an
/// `options` param (opens the panel on that style's Style Options). Every other panel id is a panel id of
/// `panel_docking::command`.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "edit.checkSpelling" && params.get("action").is_none() && params.get("ui").is_none_or(|u| u.as_bool() != Some(false)) {
        // A document without any type layer opens no dialog and shows nothing, as in the reference (#14172).
        if app.session.active().is_some_and(|st| !photocraft_engine::type_extra_cmds::has_type_layers(&st.doc)) {
            return Some(app.session.execute("edit.checkSpelling", json!({"action": "list"})).map_err(|e| e.to_string()));
        }
        // An open dialog keeps its own state. Off needs a layer to check, so without one the
        // dialog opens on, as before.
        let p = &app.ui.type_panels;
        let has_layer = app.session.active().is_some_and(|st| st.active_layer.is_some());
        let last = p.spell.as_ref().map(|d| d.all_layers).or(p.spell_all_layers).filter(|all| *all || has_layer);
        let all_layers = params.get("allLayers").and_then(Value::as_bool).or(last).unwrap_or(true);
        return Some(open_spelling(app, all_layers));
    }
    let paragraph = match id {
        "type.panels.characterStyles" | "window.panel.characterStyles" => false,
        "type.panels.paragraphStyles" | "window.panel.paragraphStyles" => true,
        _ => return None,
    };
    let style = params.get("options").and_then(Value::as_u64)?;
    app.ui.type_panels.options = Some((paragraph, style as u32));
    let panel = if paragraph { "paragraphStyles" } else { "characterStyles" };
    Some(crate::panel_docking::show_panel(app, panel, Some(true)).map(|visible| json!({ "visible": visible })))
}

fn run(app: &mut PhotocraftApp, id: &str, p: Value) -> Option<Value> {
    match app.run(id, p) {
        Ok(v) => Some(v),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            None
        }
    }
}

/// The type the panels act on: the layer being edited (with its selection), else the selected
/// layers. Returns params with `layer` + `range`, or `{}` (selected layers).
fn target(app: &PhotocraftApp) -> Value {
    if let Some(ed) = &app.ui.text_edit {
        let (a, b) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor));
        return json!({ "layer": ed.layer, "range": [a, b], "coalesce": ed.session });
    }
    json!({})
}

#[cfg(test)]
#[path = "type_spell_enabled_ui_tests.rs"]
mod spell_enabled_ui_tests;
