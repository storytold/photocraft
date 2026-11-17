//! New Set…, Insert Stop… and the Stop message: the Actions panel's own dialogs.

use craft::automation::ui::actions::ActionSelection;
use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, DialogResult, PromptAnswer, PromptChoice, StandardPrompt};
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use photocraft_engine::{actions_cmds, actions_edit_cmds, atn_cmds};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params::{self, ParamFamily};

/// The key of the New Set… dialog.
const NEW_SET: &str = "actionsPanel.newSet";
/// The key of the Insert Stop… dialog.
const INSERT_STOP: &str = "actionsPanel.insertStop";

/// The New Set… dialog's result handler.
pub(crate) const NEW_SET_FAMILY: ParamFamily = ParamFamily::new(|key| key == NEW_SET, run_new_set);
/// The Insert Stop… dialog's result handler.
pub(crate) const INSERT_STOP_FAMILY: ParamFamily = ParamFamily::new(|key| key == INSERT_STOP, run_insert_stop);

/// Shows `error` in the status bar, as every refused Actions command does.
pub(super) fn report(app: &mut PhotocraftApp, error: String) {
    if error != "cancelled" {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

/// New Set…: names an empty set folder, by default the first free "Set N".
pub(super) fn new_set_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    let names = actions_edit_cmds::set_names(&app.session);
    let name = (1u32..).map(|n| format!("Set {n}")).find(|n| !names.contains(n)).unwrap_or_default();
    let id = params::open_param(app, NEW_SET, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
        let rows = vec![Param::new("name", tl!("Name:"), ParamKind::text(name.clone()))];
        ParamDialog::command(NEW_SET, tl!("New Set"), rows).values(Bind::bound(fields))
    });
    Ok(json!({"dialog": id}))
}

/// Makes the set and selects it, so New Action records into it.
fn run_new_set(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    match app.run("actions.newSet", json!({"name": result.fields.text("name")})) {
        Ok(made) => {
            if let Some(set) = made["set"].as_str() {
                app.ui.actions.panel.selected = Some(ActionSelection::Set(set.to_owned()));
            }
        }
        Err(error) => report(app, error),
    }
}

/// The action the panel's selection is in, when the list still has it.
fn selected_action(app: &PhotocraftApp) -> Option<(usize, Option<usize>)> {
    let (action, step) = match app.ui.actions.panel.selected.as_ref()? {
        ActionSelection::Action(action) => (*action, None),
        ActionSelection::Step(action, step) => (*action, Some(*step)),
        _ => return None,
    };
    app.session.actions.list.action(action).map(|_| (action, step))
}

/// Insert Stop…: a message step after the selected step, or at the end of the selected action (pending steps of a
/// recording count). The action and the place are captured here; the dialog shows neither.
pub(super) fn insert_stop_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    let (action, step) = selected_action(app).ok_or_else(|| tl!("Select an action first").to_string())?;
    let len = app.session.actions.list.step_count(action) + actions_cmds::pending_steps(&app.session, action).count();
    let at = step.map_or(len, |step| (step + 1).min(len));
    let id = params::open_param(app, INSERT_STOP, DialogFields::new(), json!({"action": action, "at": at}), |fields: &mut DialogFields| {
        let rows = vec![
            Param::new("message", tl!("Message:"), ParamKind::text("")).multiline(4),
            Param::new("allowContinue", tl!("Allow Continue"), ParamKind::bool(false)),
        ];
        ParamDialog::command(INSERT_STOP, tl!("Insert Stop"), rows).values(Bind::bound(fields))
    });
    Ok(json!({"dialog": id}))
}

/// Inserts the Stop step into the captured action, selects it and opens its action.
fn run_insert_stop(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let captured = params::captured(app, INSERT_STOP);
    let Some(action) = captured["action"].as_u64().and_then(|action| usize::try_from(action).ok()) else {
        return report(app, tl!("The dialog names no action").to_string());
    };
    let request = json!({
        "action": action,
        "at": captured["at"],
        "id": atn_cmds::STOP_MESSAGE,
        "params": {
            "message": result.fields.text("message").unwrap_or_default(),
            "allowContinue": result.fields.bool("allowContinue").unwrap_or(false),
        },
    });
    match app.run("actions.insert", request) {
        Ok(inserted) => {
            let Some(step) = inserted["step"].as_u64().and_then(|step| usize::try_from(step).ok()) else { return };
            app.ui.actions.panel.selected = Some(ActionSelection::Step(action, step));
            if let Some(held) = app.session.actions.list.action(action) {
                app.ui.actions.panel.expanded.insert(held.name.clone());
            }
        }
        Err(error) => report(app, error),
    }
}

/// Tells a Stop message's answer which paused playback it belongs to.
struct StopTag {
    token: u64,
}

/// The message of a Stop step shown to a person: Continue and Stop, or Stop alone when the step does not allow
/// continuing. The host keeps its result apart (it is an instance), so [`take_stops`] takes it. The key router, not this prompt, keeps the Enter
/// that started the play from answering it.
pub(super) fn open_stop_message(app: &mut PhotocraftApp, message: &str, allow_continue: bool, token: u64) -> DialogId {
    let prompt = StandardPrompt::custom(message.to_owned()).title(tl!("Message"));
    let prompt = if allow_continue {
        prompt.button(PromptChoice::Proceed, tl!("Continue")).button(PromptChoice::Cancel, tl!("Stop"))
    } else {
        prompt.button(PromptChoice::Proceed, tl!("Stop"))
    };
    // One prompt per paused playback: a called action's Stop opens its own while its callers wait.
    app.framework.dialogs.open(prompt.instance(token).tag(StopTag { token }))
}

/// A `TAKERS` row: the answers of the open Stop messages. Each message is its own instance, so the host keeps its result
/// apart from the plain prompts', and the paused playback names the dialog it waits at. An answer is
/// [`answer_stop`]'s; a message closed from outside ends its playback like Stop.
pub(crate) fn take_stops(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let waiting: Vec<(u64, DialogId)> = app.paused_playbacks.paused.iter().filter_map(|(token, paused)| paused.dialog.map(|dialog| (*token, dialog))).collect();
    for (token, dialog) in waiting {
        match app.framework.dialogs.take_by_id::<StandardPrompt>(dialog) {
            Some(DialogResult::Ok(mut answer)) => {
                answer_stop(app, ctx, &mut answer);
            }
            Some(_) => super::runner::stopped(app, token),
            None => {}
        }
    }
}

/// Answers a Stop message: Continue resumes the paused playback when the step allows it, anything else ends it. What
/// the step allows is read from the paused playback, never from the prompt, so an answer cannot turn a hard stop into
/// Continue. `false` for another prompt's answer.
fn answer_stop(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(StopTag { token }) = answer.take_tag::<StopTag>() else { return false };
    let allowed = app.paused_playbacks.paused.get(&token).is_some_and(|paused| paused.allow_continue);
    if answer.choice == PromptChoice::Proceed && allowed {
        if let Err(error) = super::runner::resume(app, token) {
            report(app, error);
        }
    } else {
        super::runner::stopped(app, token);
    }
    true
}
