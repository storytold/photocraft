//! Window › Layer Comps panel (a tab of the History card).
//!
//! The rows come from the document's comps; the framework's `LayerCompsPanel` draws them and
//! reports events, and every event runs as a `layerComp.*` engine command. The selection is
//! `UiState::layer_comp_selected`.

use craft::behavior::{EditEvent, Mixed};
use craft::layers::ui::comps::{CompOption, LayerCompRow, LayerCompsEvent, LayerCompsPanel};
use craft::ui::contract::Bind;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The comp footer and row actions act on: the panel selection if still valid, else the last
/// applied comp.
pub fn target(app: &PhotocraftApp) -> Option<u32> {
    let doc = &app.session.active()?.doc;
    app.ui.layer_comp_selected.filter(|id| doc.comp(*id).is_some()).or(doc.last_applied_comp.filter(|id| doc.comp(*id).is_some()))
}

fn run(app: &mut PhotocraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The panel menu has no rows: Move Up and Move Down belong to the row menu.
pub(crate) fn panel_rows(_app: &PhotocraftApp) -> Vec<PanelRow> {
    Vec::new()
}

/// The panel has no menu commands to run.
pub(crate) fn run_fired(_app: &mut PhotocraftApp, _ctx: &egui::Context, _command: &CommandRef) -> bool {
    false
}

fn option_key(option: CompOption) -> Option<&'static str> {
    match option {
        CompOption::Visibility => Some("visibility"),
        CompOption::Position => Some("position"),
        CompOption::Appearance => Some("appearance"),
        _ => None,
    }
}

/// The commands of one option-toggle event. The events of a gesture share a coalesce key, so a
/// sweep and its restore after Escape are one undo step; a `Set` is a step of its own.
fn option_commands(comps: &[u64], option: CompOption, edit: &EditEvent<bool>) -> Vec<(&'static str, Value)> {
    let Some(key) = option_key(option) else { return Vec::new() };
    let (value, gesture) = match edit {
        EditEvent::Update { id, value, .. } => (*value, Some(id.0)),
        EditEvent::Commit { id, after, .. } => (*after, Some(id.0)),
        EditEvent::Set { after, .. } => (*after, None),
        EditEvent::Cancel { id, before: Some(Mixed::Same(v)), .. } => (*v, Some(id.0)),
        _ => return Vec::new(),
    };
    comps
        .iter()
        .map(|comp| {
            let mut p = json!({ "comp": comp, key: value });
            if let Some(g) = gesture {
                p["coalesce"] = json!(format!("comp-option-{g}"));
            }
            ("layerComp.setOptions", p)
        })
        .collect()
}

/// The Layer Comps panel.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (rows, document, last_state_applied) = match app.session.active() {
        Some(st) => {
            let doc = &st.doc;
            let rows: Vec<LayerCompRow> = doc
                .layer_comps
                .iter()
                .map(|c| {
                    LayerCompRow::new(u64::from(c.id), c.name.clone())
                        .comment(c.comment.clone())
                        .applied(doc.last_applied_comp == Some(c.id))
                        .missing(c.missing_layers(doc).len())
                        .visibility(c.apply_visibility)
                        .position(c.apply_position)
                        .appearance(c.apply_appearance)
                })
                .collect();
            (rows, Some(doc.id.0), doc.last_applied_comp.is_none())
        }
        None => (Vec::new(), None, false),
    };
    // The panel highlights the fallback target, but only a change the user made is stored as the selection.
    let shown = target(app).map(u64::from);
    let mut selected = shown;
    let mut panel = LayerCompsPanel::new(tl!("Layer Comps"), &rows, Bind::bound(&mut selected)).last_state_applied(last_state_applied);
    panel = match document {
        Some(document) => panel.document(document),
        None => panel.empty(tl!("No document")),
    };
    let events = panel.show(ui).events;
    if selected != shown {
        app.ui.layer_comp_selected = selected.map(|id| id as u32);
    }

    for event in events {
        match event {
            LayerCompsEvent::RestoreLastDocumentState => {
                let can = app.session.active().is_some_and(|s| s.doc.last_document_state.is_some() && s.doc.last_applied_comp.is_some());
                if can {
                    run(app, "layerComp.restoreLastDocumentState", json!({}));
                }
            }
            LayerCompsEvent::Select(id) => app.ui.layer_comp_selected = Some(id as u32),
            LayerCompsEvent::Apply(comp) => run(app, "layerComp.apply", json!({ "comp": comp })),
            LayerCompsEvent::Rename { comp, name, .. } => run(app, "layerComp.rename", json!({ "comp": comp, "name": name })),
            LayerCompsEvent::Reorder { comp, index, .. } => {
                app.ui.layer_comp_selected = Some(comp as u32);
                run(app, "layerComp.reorder", json!({ "comp": comp, "index": index }));
            }
            LayerCompsEvent::SetOption { comps, option, edit, .. } => {
                for (id, p) in option_commands(&comps, option, &edit) {
                    run(app, id, p);
                }
            }
            LayerCompsEvent::ClearWarning(comp) => run(app, "layerComp.updateWarnings", json!({ "clear": true, "comp": comp })),
            LayerCompsEvent::Previous => run(app, "layerComp.previous", json!({})),
            LayerCompsEvent::Next => run(app, "layerComp.next", json!({})),
            LayerCompsEvent::Update(comp) => run(app, "layerComp.update", json!({ "comp": comp })),
            LayerCompsEvent::New => {
                run(app, "layerComp.new", json!({}));
                app.ui.layer_comp_selected = app.session.active().and_then(|d| d.doc.last_applied_comp);
            }
            LayerCompsEvent::Delete(comp) => run(app, "layerComp.delete", json!({ "comp": comp })),
            _ => {}
        }
    }
}
