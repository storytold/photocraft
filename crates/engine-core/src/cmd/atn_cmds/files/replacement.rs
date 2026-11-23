//! Replace Actions…: the whole library becomes every set of an ATN archive, prepared completely
//! before the list changes.
use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

use crate::actions_cmds::{ActionList, ActionsFile, PLAY_SHORTCUT_PREFIX};
use crate::actions_edit_cmds::{confirmed_idle, publish_library};
use crate::preset_import_cmds::UniqueNames;
use crate::{Result, Session};

use super::super::bad;
use super::super::convert::{ReplacementSet, Unsupported, import_sets};

const COMMAND: &str = "actions.replace";

/// The archive as one library: each set under a name unique among the archive's sets, each action
/// under a name unique across the archive, and the calls between its actions pointed at those
/// names, so the list takes them as they are.
struct Replacement {
    file: ActionsFile,
    sets: Vec<String>,
    keys: BTreeMap<String, String>,
    warnings: Vec<String>,
    unsupported: Vec<Unsupported>,
}

impl Replacement {
    fn parse(bytes: &[u8], encoding: Option<&str>) -> Result<Self> {
        let mut actions = Vec::new();
        let mut sets = Vec::new();
        let mut keys = BTreeMap::new();
        let mut warnings = Vec::new();
        let mut unsupported = Vec::new();
        let mut set_names = UniqueNames::new_exact(std::iter::empty::<&str>());
        let mut action_names = UniqueNames::new_exact(std::iter::empty::<&str>());
        // Name the archive gave an action → the name it gets: the first of that name anywhere, the
        // first in a set of the archive's naming, and the first in the same set.
        let mut first = HashMap::new();
        let mut scoped = HashMap::new();
        let mut local = HashMap::new();
        // Each action's set in the archive, by its new name: (set index, set name in the archive).
        let mut origin = Vec::new();
        for (ReplacementSet { mut imported, keys: function_keys }, unsupported_rows) in import_sets(bytes, encoding).map_err(|error| bad(COMMAND, error))? {
            let slot = sets.len();
            let set = set_names.unique(&imported.set);
            for action in &mut imported.actions {
                let name = action_names.unique(&action.name);
                let original = std::mem::replace(&mut action.name, name);
                first.entry(original.clone()).or_insert_with(|| action.name.clone());
                scoped.entry((imported.set.clone(), original.clone())).or_insert_with(|| action.name.clone());
                local.entry((slot, original)).or_insert_with(|| action.name.clone());
                action.set = Some(set.clone());
                origin.push((slot, imported.set.clone()));
            }
            for (index, key) in function_keys {
                if let Some(action) = imported.actions.get(index) {
                    // A function key belongs to one action: the last the archive gives it.
                    keys.retain(|_, assigned| assigned != &key);
                    keys.insert(format!("{PLAY_SHORTCUT_PREFIX}{}", action.name), key);
                }
            }
            for (diagnostic, row) in imported.unsupported.iter_mut().zip(unsupported_rows) {
                diagnostic.action.clone_from(&imported.actions[row].name);
            }
            actions.extend(imported.actions);
            sets.push(set);
            warnings.extend(imported.warnings);
            unsupported.extend(imported.unsupported);
        }
        let set_of: HashMap<String, Option<String>> = actions.iter().map(|action| (action.name.clone(), action.set.clone())).collect();
        for (action, (slot, archive_set)) in actions.iter_mut().zip(&origin) {
            for step in &mut action.steps {
                let (id, params) = (step.command.as_str(), &mut step.params);
                let named_automation = matches!(id, "file.automate.batch" | "file.automate.createDroplet");
                let fields: &[(&str, &str)] = match id {
                    "actions.play" => &[("action", "set")],
                    "actions.conditional" => &[("then", "thenSet"), ("else", "elseSet")],
                    _ if named_automation && params.get("steps").is_none() => &[("action", "set")],
                    _ => &[],
                };
                for (field, scope_key) in fields {
                    let Some(name) = params.get(*field).and_then(Value::as_str) else { continue };
                    let scope = params.get(*scope_key).filter(|value| !value.is_null()).or_else(|| params.get("set").filter(|value| !value.is_null()));
                    // An unqualified call looks in its own set first, then anywhere in the archive.
                    let renamed = match scope {
                        None => local.get(&(*slot, name.to_owned())).or_else(|| first.get(name)),
                        Some(value) => value.as_str().and_then(|set| {
                            let own = (set == archive_set).then(|| local.get(&(*slot, name.to_owned()))).flatten();
                            own.or_else(|| scoped.get(&(set.to_owned(), name.to_owned())))
                        }),
                    };
                    let Some(renamed) = renamed.cloned() else { continue };
                    // The call names its target's set as the list now has it.
                    if scope.is_some()
                        && let Some(Some(set)) = set_of.get(&renamed)
                    {
                        let key = if params.get(*scope_key).is_some_and(|value| !value.is_null()) { *scope_key } else { "set" };
                        params[key] = json!(set);
                    }
                    params[*field] = json!(renamed);
                }
            }
        }
        Ok(Self { file: ActionsFile::new(actions, Vec::new()), sets, keys, warnings, unsupported })
    }
}

/// The number of actions an archive would replace the library with, or why it can't.
pub fn validate(bytes: &[u8]) -> std::result::Result<usize, String> {
    Replacement::parse(bytes, None).map(|prepared| prepared.file.actions.len()).map_err(|error| error.to_string())
}

pub(in super::super) fn replace(s: &mut Session, p: &Value) -> Result<Value> {
    confirmed_idle(s, p, COMMAND)?;
    if p.get("set").is_some() || p.get("replace").is_some() {
        return Err(bad(COMMAND, "set overrides and same-set replacement belong to actions.load"));
    }
    let encoding = match p.get("encoding") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if value == "auto" => None,
        Some(Value::String(value)) if !value.is_empty() => Some(value.as_str()),
        _ => return Err(bad(COMMAND, "encoding must be auto or a non-empty label")),
    };
    let (bytes, _) = crate::preset_import_cmds::file_bytes(p, COMMAND, ".atn file")?;
    let Replacement { mut file, sets, keys, warnings, unsupported } = Replacement::parse(&bytes, encoding)?;
    // Whole-library imports keep their automation origin just as actions.load imports do.
    for action in &mut file.actions {
        action.automation = s.authorize.is_some();
    }
    let names: Vec<String> = file.actions.iter().map(|action| action.name.clone()).collect();
    // A set the archive holds with no action stays as an empty set.
    file.sets = sets.iter().filter(|set| !file.actions.iter().any(|action| action.set.as_ref() == Some(*set))).cloned().collect();
    publish_library(s, COMMAND, Some(keys), |list: &mut ActionList| list.replace_all(file, |_, _| {}))?;
    Ok(json!({"sets": sets, "actions": names, "warnings": warnings, "unsupported": unsupported}))
}
