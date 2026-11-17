//! The Layers panel menu's Duplicate Layer… (#14615): right after New Group…, opening the same
//! dialog as Layer › Duplicate Layer…, plural with several layers selected, greyed without a document.
use craft::keymap::CommandRef;
use craft::ui::dialog::StackDialog;
use craft::ui::menu::PanelRow;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use craft::ui::runtime::commands::CommandState;
use serde_json::json;

use super::{command_states, panel_rows};
use crate::PhotocraftApp;
use crate::new_layer_ui::testing::{fields, top};

fn doc_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().map_or(0, |st| st.doc.walk().len())
}

/// The panel menu's rows, by command id; a separator is `None`.
fn row_ids(app: &PhotocraftApp) -> Vec<Option<String>> {
    panel_rows(app)
        .into_iter()
        .filter_map(|row| match row {
            PanelRow::Command(c) => Some(Some(c.id.clone())),
            PanelRow::Separator => Some(None),
            _ => None,
        })
        .collect()
}

/// Runs Duplicate Layer… as the panel row and Layer › Duplicate Layer… do; the `name` field of the dialog it opens.
fn open_dialog(app: &mut PhotocraftApp) -> Option<String> {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, "layer.duplicate", json!({})).unwrap();
    crate::dialogs::testing::show(app);
    let id = top(app);
    let ours = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_some_and(|d| StackDialog::key(d).is_some_and(crate::duplicate_layer_ui::owns));
    assert!(ours, "the Duplicate Layer dialog");
    fields(app, id).text("name").map(str::to_string)
}

/// The state the panel publishes for Duplicate Layer…, if any.
fn duplicate_state(app: &PhotocraftApp) -> Option<CommandState> {
    command_states(app).into_iter().find(|(c, _)| *c == CommandRef::new("layer.duplicate")).map(|(_, s)| s)
}

#[test]
fn duplicate_layer_comes_right_after_new_group() {
    let ids = row_ids(&doc_app());
    let at = |id: &str| ids.iter().position(|r| r.as_deref() == Some(id)).unwrap_or_else(|| panic!("no row {id}"));
    let (layer, group, duplicate, from_layers) = (at("layer.new.layer"), at("layer.new.group"), at("layer.duplicate"), at("layer.new.groupFromLayers"));
    assert!(layer < group, "New Layer… before New Group…");
    assert_eq!(duplicate, group + 1, "nothing sits between New Group… and Duplicate Layer…");
    assert_eq!(from_layers, duplicate + 1, "nor between it and New Group from Layers…");
}

#[test]
fn duplicate_layer_opens_the_same_dialog_as_the_layer_menu() {
    let mut app = doc_app();
    let before = layer_count(&app);
    assert_eq!(open_dialog(&mut app).as_deref(), Some("Ink copy"));
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(layer_count(&app), before, "nothing is duplicated before OK");
}

#[test]
fn several_selected_layers_read_duplicate_layers() {
    let mut app = doc_app();
    assert_eq!(duplicate_state(&app), None, "one layer: the command's own label");
    let st = app.session.active().unwrap();
    let background = st.doc.walk().into_iter().map(|(_, _, l)| l.id.0).find(|id| Some(*id) != st.active_layer.map(|a| a.0)).unwrap();
    app.run("layer.select", json!({"layer": background, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    let expected = CommandState::new().enabled(true).title("Duplicate Layers…");
    assert_eq!(duplicate_state(&app), Some(expected));
    assert_eq!(open_dialog(&mut app), None, "several layers keep their names: no As field");
}

#[test]
fn duplicate_layer_is_greyed_without_a_document() {
    let no_doc = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(!crate::menus::is_enabled(&no_doc, "layer.duplicate"), "greyed without a document");
    assert!(crate::menus::is_enabled(&doc_app(), "layer.duplicate"));
}
