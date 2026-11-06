//! Prepare the complete archive before publishing a whole Actions library.
use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

use crate::actions_cmds::{Action, PLAY_SHORTCUT_PREFIX};
use crate::actions_edit_cmds::{check_library_size, confirmed_idle, publish_library};
use crate::{Result, Session};

use super::super::bad;
use super::super::convert::{ReplacementSet, UniqueNames, Unsupported, import_sets};

const COMMAND: &str = "actions.replace";

struct Replacement {
    list: Vec<Action>,
    rows: Vec<String>,
    keys: BTreeMap<String, String>,
    warnings: Vec<String>,
    unsupported: Vec<Unsupported>,
}

impl Replacement {
    fn parse(bytes: &[u8], encoding: Option<&str>) -> Result<Self> {
        let mut list = Vec::new();
        let mut rows = Vec::new();
        let mut keys = BTreeMap::new();
        let mut warnings = Vec::new();
        let mut unsupported = Vec::new();
        let mut names = UniqueNames::new(std::iter::empty::<String>());
        let mut first = HashMap::new();
        let mut scoped = HashMap::new();
        let mut local = HashMap::new();
        for ReplacementSet { mut imported, keys: function_keys } in import_sets(bytes, encoding).map_err(|error| bad(COMMAND, error))? {
            let slot = rows.len();
            for action in &mut imported.actions {
                let name = names.claim(&action.name);
                let original = std::mem::replace(&mut action.name, name);
                first.entry(original.clone()).or_insert_with(|| action.name.clone());
                scoped.entry((imported.set.clone(), original.clone())).or_insert_with(|| action.name.clone());
                local.entry((slot, original)).or_insert_with(|| action.name.clone());
                action.set_slot = Some(slot);
            }
            for (index, key) in function_keys {
                if let Some(action) = imported.actions.get(index) {
                    keys.retain(|_, assigned| assigned != &key);
                    keys.insert(format!("{PLAY_SHORTCUT_PREFIX}{}", action.name), key);
                }
            }
            list.extend(imported.actions);
            rows.push(imported.set);
            warnings.extend(imported.warnings);
            unsupported.extend(imported.unsupported);
        }
        for action in &mut list {
            for (id, params) in &mut action.steps {
                let fields: &[&str] = match id.as_str() {
                    "actions.play" => &["action"],
                    "actions.conditional" => &["then", "else"],
                    "file.automate.batch" if params.get("steps").is_none() => &["action"],
                    _ => &[],
                };
                for field in fields {
                    let Some(name) = params.get(*field).and_then(Value::as_str) else { continue };
                    let scope = match *field {
                        "then" => params.get("thenSet").filter(|value| !value.is_null()),
                        "else" => params.get("elseSet").filter(|value| !value.is_null()),
                        _ => None,
                    }
                    .or_else(|| params.get("set").filter(|value| !value.is_null()));
                    // An unqualified call in this archive first looks in its original folder.
                    // Legacy callers outside this import still use their existing name lookup.
                    let renamed = match scope {
                        None => action.set_slot.and_then(|slot| local.get(&(slot, name.to_string()))).or_else(|| first.get(name)),
                        Some(value) => value.as_str().and_then(|set| {
                            let row = action.set_slot.filter(|_| action.set.as_deref() == Some(set));
                            row.and_then(|slot| local.get(&(slot, name.to_string()))).or_else(|| scoped.get(&(set.to_string(), name.to_string())))
                        }),
                    };
                    if let Some(renamed) = renamed {
                        params[*field] = json!(renamed);
                    }
                }
            }
        }
        check_library_size(&list, &[], &rows, COMMAND)?;
        Ok(Self { list, rows, keys, warnings, unsupported })
    }
}

pub fn validate(bytes: &[u8]) -> std::result::Result<usize, String> {
    Replacement::parse(bytes, None).map(|prepared| prepared.list.len()).map_err(|error| error.to_string())
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
    let Replacement { list, rows, keys, warnings, unsupported } = Replacement::parse(&bytes, encoding)?;
    let result = json!({"sets":rows,"actions":list.iter().map(|a| &a.name).collect::<Vec<_>>(),"warnings":warnings,"unsupported":unsupported});
    publish_library(s, list, Vec::new(), rows, Some(keys), "Undo Replace Actions", COMMAND)?;
    Ok(result)
}
