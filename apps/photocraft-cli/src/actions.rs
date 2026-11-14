use std::{
    io::Read,
    path::{Path, PathBuf},
};

use photocraft_engine::actions_cmds::Action;
use serde_json::{Value, json};

/// Why an actions file can't be played.
#[derive(Debug, thiserror::Error)]
pub enum ActionsError {
    #[error("actions JSON: {0}")]
    Json(#[source] serde_json::Error),
    /// The steps don't parse ([`photocraft_engine::automate_cmds::parse_action`]).
    #[error(transparent)]
    Steps(#[from] photocraft_engine::EngineError),
    #[error("--action selects an action from an ATN library; a droplet already holds one action")]
    NotASet,
    #[error("legacy JSON action libraries are no longer supported; use an ATN library or a droplet")]
    LegacyLibrary,
    #[error("Actions set: {0}")]
    Set(#[source] serde_json::Error),
    #[error("no action named `{name}` in the set (it holds {})", quoted(names))]
    NoSuchAction { name: String, names: Vec<String> },
    #[error("the Actions set holds {} actions: pick one with --action <name> ({})", names.len(), quoted(names))]
    PickOne { names: Vec<String> },
    #[error("{}: {source}", path.display())]
    Read { path: PathBuf, source: std::io::Error },
    #[error("{}: {source}", path.display())]
    Utf8 { path: PathBuf, source: std::string::FromUtf8Error },
    /// A Photoshop Actions file that doesn't import ([`photocraft_engine::atn_cmds::import`]).
    #[error("{}: {message}", path.display())]
    Atn { path: PathBuf, message: String },
    #[error("no action named `{name}` in the set `{set}` (it holds {})", quoted(names))]
    NoSuchAtnAction { name: String, set: String, names: Vec<String> },
    #[error("action name `{name}` is ambiguous across sets ({}); choose a unique action name", quoted(sets))]
    AmbiguousAtnAction { name: String, sets: Vec<String> },
    #[error("{}: the set holds no actions", path.display())]
    EmptyAtn { path: PathBuf },
    #[error("the set `{set}` holds {} actions: pick one with --action <name> ({})", names.len(), quoted(names))]
    PickOneAtn { set: String, names: Vec<String> },
}

/// `` `a`, `b` ``: the action names an error lists.
fn quoted(names: &[String]) -> String {
    names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
}

fn names(actions: &[Action]) -> Vec<String> {
    actions.iter().map(|a| a.name.clone()).collect()
}

/// Parse an actions file: the steps of a recorded action or a droplet (`[id, params]` pairs,
/// `{"command": id, "params": {…}}` objects or bare ids; `"id"` is accepted for `"command"`), as a
/// list or wrapped in `{"actions": […]}`, `{"steps": […]}` or a droplet (#489).
pub fn parse_actions(text: &str) -> Result<Vec<(String, Value)>, ActionsError> {
    let v: Value = serde_json::from_str(text).map_err(ActionsError::Json)?;
    steps_of(&v)
}

/// Read the actual step array from a droplet/action value and report malformed input before Batch starts.
fn steps_of(v: &Value) -> Result<Steps, ActionsError> {
    Ok(photocraft_engine::automate_cmds::parse_action(v.get("actions").unwrap_or(v), "batch --actions")?)
}

/// Engine commands to run in order: `(id, params)`.
pub type Steps = Vec<(String, Value)>;

/// Parse one droplet/step value. A legacy JSON action library is never imported here.
pub fn load_actions(text: &str, name: Option<&str>) -> Result<(Steps, Vec<Action>), ActionsError> {
    let v: Value = serde_json::from_str(text).map_err(ActionsError::Json)?;
    let list = v.get("actions").unwrap_or(&v);
    if list.as_array().is_some_and(|list| list.iter().any(|action| action.get("name").is_some())) {
        return Err(ActionsError::LegacyLibrary);
    }
    if name.is_some() {
        return Err(ActionsError::NotASet);
    }
    let called = match v.get("calledActions") {
        None | Some(Value::Null) => Vec::new(),
        Some(called) => serde_json::from_value(called.clone()).map_err(ActionsError::Set)?,
    };
    Ok((steps_of(&v)?, called))
}

/// [`load_actions`] for a file on disk: a Photoshop Actions file (`.atn`, by extension or by
/// its version word, 12 or 16) is converted first; the played action's steps with no PhotoCraft
/// equivalent come back as warnings (they fail at playback, as in Photoshop when a step can't run).
pub fn load_actions_file(path: &Path, name: Option<&str>) -> Result<(Steps, Vec<Action>, Vec<String>), ActionsError> {
    let file = std::fs::File::open(path).map_err(|source| ActionsError::Read { path: path.to_path_buf(), source })?;
    let mut bytes = Vec::new();
    // CLI batch input is an external action file, not the smaller persisted preset store.
    let limit = photocraft_engine::preset_import_cmds::MAX_FILE_BYTES;
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|source| ActionsError::Read { path: path.to_path_buf(), source })?;
    if bytes.len() as u64 > limit {
        return Err(ActionsError::Read {
            path: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, "Actions input exceeds byte limit"),
        });
    }
    let is_atn = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("atn")) || matches!(bytes.get(..4), Some([0, 0, 0, 12 | 16]));
    if !is_atn {
        let text = String::from_utf8(bytes).map_err(|source| ActionsError::Utf8 { path: path.to_path_buf(), source })?;
        let value: Value = serde_json::from_str(&text).map_err(ActionsError::Json)?;
        if value.get("photocraftDroplet").and_then(Value::as_u64) != Some(1)
            || !value.get("action").and_then(|action| action.get("steps")).is_some_and(Value::is_array)
        {
            return Err(ActionsError::LegacyLibrary);
        }
        let (steps, set) = load_actions(&text, name)?;
        return Ok((steps, set, Vec::new()));
    }
    let mut imp = photocraft_engine::atn_cmds::import(&bytes).map_err(|message| ActionsError::Atn { path: path.to_path_buf(), message })?;
    let selected = match (name, imp.actions.as_slice()) {
        (Some(n), _) => {
            let matched: Vec<_> = imp.actions.iter().enumerate().filter(|(_, action)| action.name == n).collect();
            match matched.as_slice() {
                [(index, _)] => *index,
                [] => return Err(ActionsError::NoSuchAtnAction { name: n.to_owned(), set: imp.set.clone(), names: names(&imp.actions) }),
                _ => {
                    return Err(ActionsError::AmbiguousAtnAction {
                        name: n.to_owned(),
                        sets: matched.iter().map(|(_, action)| action.set.clone().unwrap_or_else(|| imp.set.clone())).collect(),
                    });
                }
            }
        }
        (None, [_]) => 0,
        (None, []) => return Err(ActionsError::EmptyAtn { path: path.to_path_buf() }),
        (None, _) => return Err(ActionsError::PickOneAtn { set: imp.set.clone(), names: names(&imp.actions) }),
    };
    let action = &imp.actions[selected];
    let warnings = imp
        .unsupported
        .iter()
        .filter(|u| u.action == action.name)
        .map(|u| format!("`{}` step {}: Photoshop's \"{}\" ({}) has no PhotoCraft equivalent yet", u.action, u.step + 1, u.name, u.event))
        .chain(imp.warnings.iter().cloned())
        .collect();
    photocraft_engine::atn_cmds::normalize_import(&mut imp, std::iter::empty(), false)
        .map_err(|message| ActionsError::Atn { path: path.to_path_buf(), message })?;
    let play = vec![("actions.play".to_string(), json!({"action": imp.actions[selected].name}))];
    Ok((play, imp.actions, warnings))
}
