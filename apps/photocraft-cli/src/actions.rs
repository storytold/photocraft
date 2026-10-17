use std::path::Path;

use photocraft_engine::actions_cmds::Action;
use serde_json::{Value, json};

/// Parse an actions file: the steps of a recorded action or a droplet (`[id, params]` pairs,
/// `{"command": id, "params": {…}}` objects or bare ids; `"id"` is accepted for `"command"`), as a
/// list or wrapped in `{"actions": […]}`, `{"steps": […]}` or a droplet (#489).
pub fn parse_actions(text: &str) -> Result<Vec<(String, Value)>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("actions JSON: {e}"))?;
    steps_of(&v)
}

fn steps_of(v: &Value) -> Result<Steps, String> {
    photocraft_engine::automate_cmds::parse_action(v.get("actions").unwrap_or(v), "batch --actions").map_err(|e| e.to_string())
}

/// Engine commands to run in order: `(id, params)`.
pub type Steps = Vec<(String, Value)>;

/// The steps `batch` plays on each file, and the actions they can call. An Actions set (the
/// Actions panel's `actions.json`: `{"actions": [{"name", "steps"}…]}`, or the bare list) plays
/// the action `name` picks (optional when the set holds one) with the whole set loaded, as
/// Photoshop's Batch does; anything else is one action's steps ([`parse_actions`]) (#2786).
pub fn load_actions(text: &str, name: Option<&str>) -> Result<(Steps, Vec<Action>), String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("actions JSON: {e}"))?;
    let list = v.get("actions").unwrap_or(&v);
    let is_set = list.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|x| x.get("name").is_some_and(Value::is_string)));
    if !is_set {
        if name.is_some() {
            return Err("--action picks an action from an Actions set ({\"actions\": [{\"name\", \"steps\"}…]}), but this file holds one action's steps".into());
        }
        return Ok((steps_of(&v)?, Vec::new()));
    }
    let set: Vec<Action> = serde_json::from_value(list.clone()).map_err(|e| format!("Actions set: {e}"))?;
    let names = || set.iter().map(|a| format!("`{}`", a.name)).collect::<Vec<_>>().join(", ");
    let action = match (name, set.as_slice()) {
        (Some(n), _) => set.iter().find(|a| a.name == n).ok_or_else(|| format!("no action named `{n}` in the set (it holds {})", names()))?,
        (None, [only]) => only,
        (None, _) => return Err(format!("the Actions set holds {} actions: pick one with --action <name> ({})", set.len(), names())),
    };
    let play = vec![("actions.play".to_string(), json!({"action": action.name}))];
    Ok((play, set))
}

/// [`load_actions`] for a file on disk: a Photoshop Actions file (`.atn`, by extension or by
/// its version word) is converted first; the played action's steps with no PhotoCraft
/// equivalent come back as warnings (they fail at playback, as in Photoshop when a step can't run).
pub fn load_actions_file(path: &str, name: Option<&str>) -> Result<(Steps, Vec<Action>, Vec<String>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let is_atn = Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("atn")) || bytes.starts_with(&[0, 0, 0, 16]);
    if !is_atn {
        let text = String::from_utf8(bytes).map_err(|e| format!("{path}: {e}"))?;
        let (steps, set) = load_actions(&text, name)?;
        return Ok((steps, set, Vec::new()));
    }
    let imp = photocraft_engine::atn_cmds::import(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let names = || imp.actions.iter().map(|a| format!("`{}`", a.name)).collect::<Vec<_>>().join(", ");
    let action = match (name, imp.actions.as_slice()) {
        (Some(n), _) => {
            imp.actions.iter().find(|a| a.name == n).ok_or_else(|| format!("no action named `{n}` in the set `{}` (it holds {})", imp.set, names()))?
        }
        (None, [only]) => only,
        (None, []) => return Err(format!("{path}: the set holds no actions")),
        (None, _) => return Err(format!("the set `{}` holds {} actions: pick one with --action <name> ({})", imp.set, imp.actions.len(), names())),
    };
    let warnings = imp
        .unsupported
        .iter()
        .filter(|u| u.action == action.name)
        .map(|u| format!("`{}` step {}: Photoshop's \"{}\" ({}) has no PhotoCraft equivalent yet", u.action, u.step + 1, u.name, u.event))
        .chain(imp.warnings.iter().cloned())
        .collect();
    let play = vec![("actions.play".to_string(), json!({"action": action.name}))];
    Ok((play, imp.actions, warnings))
}
