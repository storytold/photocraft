//! The Count tool's "Count Group Name" dialog (#11766, #14707): the options bar's folder button
//! opens it to name a new count group, and the group menu's Rename… opens it to rename the current
//! one, as in Photoshop. It is a one-field form on the dialog host: OK runs `count.newGroup` or
//! `count.setGroup` with the typed name, Cancel and Escape run nothing.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The dialog's key.
pub const KIND: &str = "countGroupName";

/// Whether the hosted dialog with `key` is the Count Group Name dialog.
pub(crate) fn owns(key: &str) -> bool {
    key == KIND
}

/// Opens the dialog for a new group, pre-filled with the name the next group gets
/// ("Count Group N", N = the groups so far + 1, as `count.newGroup` names it).
pub fn open_new(app: &mut PhotocraftApp) -> Option<DialogId> {
    let n = app.session.active()?.doc.measurement.count_groups.len();
    let name = format!("{} {}", tl!("Count Group"), n + 1);
    Some(open(app, json!({"mode": "new"}), name))
}

/// Opens the dialog to rename count group `group`, pre-filled with its name.
pub fn open_rename(app: &mut PhotocraftApp, group: usize) -> Option<DialogId> {
    let name = app.session.active()?.doc.measurement.count_groups.get(group)?.name.clone();
    Some(open(app, json!({"mode": "rename", "group": group}), name))
}

fn open(app: &mut PhotocraftApp, mut captured: Value, name: String) -> DialogId {
    // The document activation that opened it: the group index and the default name mean something only there. A
    // switch to another document (and back, or a reopened one with a reused id) makes a new activation.
    captured["gen"] = json!(app.session.active_generation());
    // Renaming targets a group by its index, which any later change to the document (a group deleted, another
    // added) can move to a different group, even with the same names.
    captured["revision"] = json!(app.session.active().map(|d| d.revision));
    params::open_param_unsaved(app, KIND, DialogFields::new(), captured, move |fields: &mut DialogFields| {
        let rows = vec![Param::new("name", tl!("Count Group Name:"), ParamKind::text(name.clone()))];
        ParamDialog::command(KIND, tl!("Count Group Name"), rows).values(Bind::bound(fields))
    })
}

/// Refuses OK once the dialog is out of date: another document has been activated since it opened, or (renaming)
/// the document has changed at all, which can move the group it targets. Nothing is named or renamed then.
pub(crate) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let captured = params::captured(app, &result.key);
    let same_activation = captured.get("gen").and_then(Value::as_u64) == Some(app.session.active_generation());
    let renaming = captured.get("mode").and_then(Value::as_str) == Some("rename");
    let same_revision = captured.get("revision").and_then(Value::as_u64) == app.session.active().map(|d| d.revision);
    if !same_activation || (renaming && !same_revision) {
        return Err(Label::from(tl!("The document changed. Open Count Group Name again.")));
    }
    Ok(())
}

/// The command OK runs: a new group with the typed name, or the current group renamed.
pub(crate) fn command(captured: &Value, fields: &DialogFields) -> (&'static str, Value) {
    let name = fields.text("name").unwrap_or_default();
    match captured.get("mode").and_then(Value::as_str) {
        Some("rename") => ("count.setGroup", json!({"group": captured.get("group"), "name": name})),
        _ => ("count.newGroup", json!({"name": name})),
    }
}

/// OK: runs the command; an engine refusal goes to the status line.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let (id, params) = command(&params::captured(app, &result.key), &result.fields);
    let outcome = app.run(id, params);
    crate::dialogs::alerts::report(app, outcome);
}
