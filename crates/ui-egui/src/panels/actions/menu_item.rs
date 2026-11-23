//! Choose an existing mapped menu event without borrowing current command settings.
use craft::automation::ui::actions::ActionSelection;
use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use photocraft_engine::actions_cmds::ActionsExt;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params::{self, ParamFamily};

const COMMAND: &str = "actions.insertMenuItem";
/// The key of the chooser dialog.
const KEY: &str = "actionsPanel.insertMenuItem";

/// The chooser's result handler. Its refusals (a changed action list, changed permissions) are checked before OK
/// closes the dialog; a Cancel releases the held target.
pub(crate) const FAMILY: ParamFamily =
    ParamFamily { check: Some(check), cancel: Some(|app, _ctx, dialog| cancel(app, dialog)), ..ParamFamily::new(|key| key == KEY, run) };

/// Keep a chooser tied to its original action, library and caller.
pub(crate) struct Held {
    dialog: DialogId,
    revision: u64,
    target: Value,
    gate: Option<super::Gate>,
    automated: bool,
}

/// Open without recording a step or sampling the selected command's current settings. The chooser inserts after
/// `at` (or at the end) of `action`.
pub(super) fn open(app: &mut PhotocraftApp, action: usize, at: Option<usize>) -> Result<Value, String> {
    // The dialog's open state is the signal: a chooser closed without a result no longer holds anything.
    if app.held_insert_menu.as_ref().is_some_and(|held| app.framework.dialogs.stack().contains(held.dialog)) {
        return Err(tl!("Confirm or cancel the pending menu item first.").to_string());
    }
    app.held_insert_menu = None;
    if app.session.actions().list.action(action).is_none() {
        return Err(tl!("Select an action first.").to_string());
    }
    let target = json!({"action": action, "at": at});
    let gate = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None });
    let automated = app.automation_input || gate.is_some();
    // Only menu identities the existing ATN event table can encode unambiguously.
    let options: Vec<(String, Label)> = photocraft_engine::atn_cmds::menu_item_commands()
        .into_iter()
        .map(|command| (command.to_string(), Label::from(super::label_of(command).to_string())))
        .collect();
    let dialog = params::open_param(app, KEY, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
        let rows = vec![Param::new("id", tl!("Command"), ParamKind::choice(options.clone(), Some("image.imageSize".into()))).searchable()];
        ParamDialog::command(KEY, tl!("Insert Menu Item"), rows).values(Bind::bound(fields))
    });
    app.held_insert_menu = Some(Held { dialog, revision: app.session.actions().list.revision(), target, gate, automated });
    Ok(json!({"dialog": dialog.value()}))
}

/// The insertion the chooser's answer asks for, once the chooser, its action list and its authority are unchanged:
/// the captured target with the picked command. Consent inserts only into the captured action and uses both the
/// retained and the current authority.
fn vet(app: &PhotocraftApp, result: &ParamResult) -> Result<Value, String> {
    let held = app
        .held_insert_menu
        .as_ref()
        .filter(|held| held.dialog == result.id && result.key == KEY)
        .ok_or_else(|| tl!("No menu item belongs to this chooser.").to_string())?;
    if held.revision != app.session.actions().list.revision() {
        return Err(tl!("The action or insertion command changed. Request Insert Menu Item again.").to_string());
    }
    let mut params = held.target.clone();
    params["id"] = json!(result.fields.text("id"));
    super::authorize_owned(app, COMMAND, &params, held.gate, held.automated)?;
    if let (Some(old), Some(current)) = (held.gate, app.session.authorize)
        && !std::ptr::fn_addr_eq(old, current)
    {
        return Err(tl!("The command permissions changed. Request Insert Menu Item again.").to_string());
    }
    Ok(params)
}

/// Refuses an answer for a changed list, another dialog or changed permissions, so the dialog stays open with the
/// reason.
fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    vet(app, result).map(|_| ()).map_err(Label::from)
}

/// Inserts the picked menu item into the captured action and selects the new step.
fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let params = match vet(app, &result) {
        Ok(params) => params,
        Err(error) => return super::dialogs::report(app, error),
    };
    let Some(held) = app.held_insert_menu.take() else { return };
    let action = params["action"].as_u64().and_then(|index| usize::try_from(index).ok());
    let saved = app.session.authorize;
    app.session.authorize = held.gate.or(saved);
    let inserted = app.run(COMMAND, params);
    app.session.authorize = saved;
    match inserted {
        Ok(inserted) => {
            let step = inserted.get("step").and_then(Value::as_u64).and_then(|index| usize::try_from(index).ok());
            if let (Some(action), Some(step)) = (action, step) {
                app.ui.actions.panel.selected = Some(ActionSelection::Step(action, step));
            }
        }
        Err(error) => super::dialogs::report(app, error),
    }
}

/// Only its own Cancel may release the chooser's target.
pub(crate) fn cancel(app: &mut PhotocraftApp, dialog: DialogId) {
    if app.held_insert_menu.as_ref().is_some_and(|held| held.dialog == dialog) {
        app.held_insert_menu = None;
    }
}
