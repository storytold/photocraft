//! Record Again executes the selected command before replacing its stored settings.
use super::{Result, Session, Value, bad, index, json, not_playing, resolve};
use crate::actions_cmds::ActionsExt;
use crate::actions_cmds::{ActionStep, list_error};

const COMMAND: &str = "actions.recordAgain";

/// A validated selection shared by the engine and the panel's seeded editor.
pub struct RecordAgainTarget {
    pub action: usize,
    pub step: usize,
    pub command: String,
    pub params: Value,
}

/// Resolve only executable steps, without flushing recording or editing the library.
pub fn target(s: &Session, p: &Value) -> Result<RecordAgainTarget> {
    not_playing(s, COMMAND)?;
    if s.recording.target.is_some() {
        return Err(bad(COMMAND, "stop recording before recording a step again"));
    }
    if let Some(revision) = p.get("revision") {
        let revision = revision.as_u64().ok_or_else(|| bad(COMMAND, "`revision` must be an unsigned integer"))?;
        if revision != s.actions().list.revision() {
            return Err(bad(COMMAND, "the actions changed; select the step again"));
        }
    }
    let action = resolve(s.actions().list.actions(), p, COMMAND)?;
    let step = index(p, "step", COMMAND)?.ok_or_else(|| bad(COMMAND, "select a step"))?;
    let ActionStep { command: id, params } =
        s.actions().list.action(action).and_then(|a| a.steps.get(step)).ok_or_else(|| bad(COMMAND, "step is out of range"))?;
    let (command, params) =
        if id == crate::atn_cmds::MENU_ITEM { (crate::atn_cmds::menu_item_command(params)?.to_string(), json!({})) } else { (id.clone(), params.clone()) };
    if command.starts_with("actions.") || !crate::commands::find(&command).is_some_and(|c| c.journal) {
        return Err(bad(COMMAND, "the selected step has no editable engine command"));
    }
    Ok(RecordAgainTarget { action, step, command, params })
}

/// Keep the original definition on invalid settings, denied execution or changed ownership.
pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let selected = target(s, p)?;
    let params = p.get("params").filter(|p| p.is_object()).cloned().ok_or_else(|| bad(COMMAND, "supply the step's new settings in `params`"))?;
    let original =
        s.actions().list.action(selected.action).and_then(|a| a.steps.get(selected.step)).cloned().ok_or_else(|| bad(COMMAND, "step is out of range"))?;
    let revision = s.actions().list.revision();
    // Resolve portable recorded targets for execution; keep their portable form in the step.
    let resolved = crate::actions_cmds::playback_params(s, &selected.command, &params)?;
    let result = s.execute(&selected.command, resolved)?;
    if s.actions().list.revision() != revision || s.actions().list.action(selected.action).and_then(|a| a.steps.get(selected.step)) != Some(&original) {
        return Err(bad(COMMAND, "the action changed during execution; its step was not overwritten"));
    }
    s.actions_mut().list.replace_step(selected.action, selected.step, ActionStep::new(selected.command.clone(), params)).map_err(|e| list_error(COMMAND, e))?;
    let name = s.actions().list.actions()[selected.action].name.clone();
    Ok(json!({"action": name, "step": selected.step, "id": selected.command, "result": result}))
}
