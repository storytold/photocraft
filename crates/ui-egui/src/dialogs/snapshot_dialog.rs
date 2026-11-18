//! History › New Snapshot (⌥-click the History panel's camera button, #2966): Photoshop's dialog
//! for `history.newSnapshot`, with the snapshot's Name and what it is taken From (the full
//! document, its layers merged, or only the current layer). The command stays directly callable
//! with the same params by agents and Actions.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Map, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The command the dialog runs; also the dialog's key.
pub const COMMAND: &str = "history.newSnapshot";

/// Whether the hosted dialog with `key` is this one.
pub(crate) fn owns(key: &str) -> bool {
    key == COMMAND
}

/// Open the dialog on the active document, suggesting the name the camera button would give.
/// Not while a Free Transform is open: it owns the document until committed or cancelled, so the
/// camera button does nothing then either (#6087).
pub fn open(app: &mut PhotocraftApp) -> Option<DialogId> {
    if app.ui.transform.is_some() {
        return None;
    }
    let name = photocraft_engine::history_cmds::next_snapshot_name(app.session.active()?);
    Some(params::open_param(app, COMMAND, DialogFields::new(), serde_json::Value::Null, move |fields: &mut DialogFields| {
        let sources: Vec<(String, Label)> = vec![
            ("fullDocument".to_string(), Label::from(tl!("Full Document"))),
            ("mergedLayers".to_string(), Label::from(tl!("Merged Layers"))),
            ("currentLayer".to_string(), Label::from(tl!("Current Layer"))),
        ];
        let rows = vec![
            Param::new("name", tl!("Name:"), ParamKind::text(name.clone())),
            Param::new("from", tl!("From:"), ParamKind::choice(sources, Some("fullDocument".to_string()))),
        ];
        ParamDialog::command(COMMAND, tl!("New Snapshot"), rows).values(Bind::bound(fields))
    }))
}

/// Takes the snapshot. A cleared Name gets the next default name, as the camera button would give
/// (#6087); agents calling the command with an empty name still get an error.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let mut p = Map::new();
    if let Some(name) = result.fields.text("name").map(str::trim).filter(|n| !n.is_empty()) {
        p.insert("name".into(), json!(name));
    }
    if let Some(from) = result.fields.text("from") {
        p.insert("from".into(), json!(from));
    }
    let outcome = app.run(COMMAND, serde_json::Value::Object(p));
    super::alerts::report(app, outcome);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::PhotocraftApp;
    use crate::new_layer_ui::testing::set;

    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app
    }

    /// An open Free Transform owns the document: New Snapshot does not open under it, whoever
    /// asks (#6087).
    #[test]
    fn does_not_open_during_free_transform() {
        let mut app = app();
        let ctx = app.ctx().clone();
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        assert!(app.ui.transform.is_some());
        assert_eq!(super::open(&mut app), None);
        assert!(app.framework.dialogs.is_empty());
        crate::transform_tool::cancel(&mut app);
        assert!(super::open(&mut app).is_some());
    }

    /// A cleared Name takes the default name on OK, as the camera button would (#6087); a typed
    /// one is trimmed. The command itself still refuses an empty name.
    #[test]
    fn a_cleared_name_gets_the_default_name() {
        let mut app = app();
        let ctx = app.ctx().clone();
        for (typed, want) in [("", "Snapshot 1"), ("   ", "Snapshot 2"), ("  Mine ", "Mine")] {
            let id = super::open(&mut app).unwrap();
            set(&mut app, id, "name", json!(typed)).unwrap();
            crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
            assert_eq!(app.session.active().unwrap().snapshots.last().unwrap().name, want);
        }
        assert!(app.run(super::COMMAND, json!({"name": ""})).is_err(), "agents still get the error");
    }
}
