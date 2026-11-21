//! The History Options dialog (History panel menu › History Options…, #2966): a form over the
//! engine's `history.options` with Photoshop's five checkboxes in its order, filled with the
//! current values. OK runs the command with the dialog's values, Cancel changes nothing. Agents can
//! also run `history.options` with params directly, or `prefs.set historyOptions.*`; a call with
//! params skips the dialog.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use photocraft_engine::history_options_cmds::{self, OPTIONS};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The command the dialog runs; also the dialog's key.
pub const COMMAND: &str = "history.options";

/// Whether the hosted dialog with `key` is this one.
pub(crate) fn owns(key: &str) -> bool {
    key == COMMAND
}

/// Opens the dialog for a parameterless `history.options`; anything else is not handled here.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != COMMAND || params.as_object().is_some_and(|m| m.keys().any(|k| !k.starts_with("__"))) {
        return None;
    }
    Some(Ok(json!({ "dialog": open(app).value() })))
}

/// Photoshop's capitalised label of option `key`, in the UI language.
fn label(key: &str) -> &'static str {
    match key {
        "automaticallyCreateFirstSnapshot" => tl!("Automatically Create First Snapshot"),
        "automaticallyCreateNewSnapshotWhenSaving" => tl!("Automatically Create New Snapshot When Saving"),
        "allowNonLinearHistory" => tl!("Allow Non-Linear History"),
        "showNewSnapshotDialogByDefault" => tl!("Show New Snapshot Dialog by Default"),
        "makeLayerVisibilityChangesUndoable" => tl!("Make Layer Visibility Changes Undoable"),
        _ => "",
    }
}

/// Open the dialog on the current options; returns its id.
pub fn open(app: &mut PhotocraftApp) -> DialogId {
    let current = history_options_cmds::current(&app.session);
    let rows: Vec<(&'static str, bool)> = OPTIONS.iter().map(|key| (*key, current.get(*key).and_then(Value::as_bool).unwrap_or(false))).collect();
    params::open_param(app, COMMAND, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
        let rows: Vec<Param> = rows.iter().map(|(key, on)| Param::new(*key, label(key), ParamKind::bool(*on))).collect();
        ParamDialog::command(COMMAND, tl!("History Options"), rows).values(Bind::bound(fields))
    })
}

/// OK: applies the five options.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let options: Map<String, Value> = OPTIONS.iter().map(|key| (key.to_string(), json!(result.fields.bool(key).unwrap_or(false)))).collect();
    let outcome = app.run(COMMAND, Value::Object(options));
    super::alerts::report(app, outcome);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::new_layer_ui::testing::{set, top};

    fn app() -> Box<PhotocraftApp> {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    #[test]
    fn the_command_opens_the_dialog_on_the_current_values_and_ok_applies_them() {
        let mut app = app();
        let ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
        let id = top(&app);
        // Cancel changes nothing.
        set(&mut app, id, "allowNonLinearHistory", json!(true)).unwrap();
        crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
        assert!(!app.session.prefs().history_options.allow_non_linear_history);
        // OK with one box flipped keeps the others at the values the dialog opened on.
        let id = open(&mut app);
        set(&mut app, id, "allowNonLinearHistory", json!(true)).unwrap();
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        let o = &app.session.prefs().history_options;
        assert!(o.allow_non_linear_history && o.make_layer_visibility_changes_undoable && o.automatically_create_first_snapshot);
        assert!(!o.automatically_create_new_snapshot_when_saving && !o.show_new_snapshot_dialog_by_default);
        // OK applies every checkbox.
        let id = open(&mut app);
        let current = history_options_cmds::current(&app.session);
        for key in OPTIONS {
            set(&mut app, id, key, json!(!current[key].as_bool().unwrap())).unwrap();
        }
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        let o = &app.session.prefs().history_options;
        assert!(!o.allow_non_linear_history && !o.make_layer_visibility_changes_undoable);
        assert!(!o.automatically_create_first_snapshot && o.automatically_create_new_snapshot_when_saving && o.show_new_snapshot_dialog_by_default);
        assert!(app.framework.dialogs.is_empty());
    }
}
