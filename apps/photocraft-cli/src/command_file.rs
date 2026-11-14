//! Preflight command files before opening a connection; never retry a dispatched command.
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use photocraft_automation::{AutomationError, BridgeClient, security};
use serde_json::{Value, json};

use crate::{
    Result,
    args::RunArgs,
    output::{JsonStyle, print_json},
};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_LINES: usize = 4096;
const MAX_LINE_BYTES: usize = 256 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_CAPTURES: usize = 256;

struct Step {
    line: usize,
    command: String,
    params: Value,
    save_as: Option<String>,
}

fn at(line: usize, cause: impl std::fmt::Display) -> String {
    format!("script line {line}: {cause}")
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn reference(text: &str) -> Result<Option<(&str, &str)>, String> {
    if !text.starts_with('$') || text.starts_with("$$") {
        return Ok(None);
    }
    let tail = text.strip_prefix('$').ok_or("invalid capture reference")?;
    let (name, pointer) = tail.find('/').map_or((tail, ""), |i| tail.split_at(i));
    if !valid_name(name) {
        return Err("capture references need $name or $name/JSON-pointer; use $$ for a literal dollar".into());
    }
    let mut chars = pointer.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return Err("JSON pointer escapes must be ~0 or ~1".into());
        }
    }
    Ok(Some((name, pointer)))
}

/// Captures are JSON data, not templates: check their depth without interpreting dollar strings.
fn check_capture_depth(value: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("params exceed nesting limit {MAX_DEPTH}"));
    }
    match value {
        Value::Array(values) => {
            for value in values {
                check_capture_depth(value, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                check_capture_depth(value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Walk both during preflight and execution, preserving the referenced JSON type.
fn template(value: &Value, captures: &HashMap<String, Value>, resolve: bool, depth: usize, budget: &mut usize) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(format!("params exceed nesting limit {MAX_DEPTH}"));
    }
    let result = match value {
        Value::String(text) => {
            if let Some((name, pointer)) = reference(text)? {
                let captured = captures.get(name).ok_or_else(|| format!("undefined or forward capture ${name}"))?;
                if resolve {
                    let found = captured.pointer(pointer).ok_or_else(|| format!("JSON pointer does not exist in capture ${name}"))?;
                    check_capture_depth(found, depth)?;
                    // Account before cloning, including repeatedly substituted large captures.
                    let size = serde_json::to_vec(found).map_err(|_| "cannot encode capture")?.len();
                    *budget = budget.checked_sub(size).ok_or("resolved params exceed 8 MiB budget")?;
                    found.clone()
                } else {
                    value.clone()
                }
            } else if resolve && text.starts_with("$$") {
                Value::String(text.strip_prefix('$').unwrap_or(text).to_owned())
            } else {
                value.clone()
            }
        }
        Value::Array(values) => Value::Array(values.iter().map(|v| template(v, captures, resolve, depth + 1, budget)).collect::<Result<_, _>>()?),
        Value::Object(values) => {
            Value::Object(values.iter().map(|(k, v)| Ok((k.clone(), template(v, captures, resolve, depth + 1, budget)?))).collect::<Result<_, String>>()?)
        }
        _ => value.clone(),
    };
    Ok(result)
}

fn load(path: &Path) -> Result<Vec<Step>, String> {
    let file = File::open(path).map_err(|e| format!("cannot open command file: {e}"))?;
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(|e| format!("cannot read command file: {e}"))?;
    if bytes.len() > MAX_BYTES {
        return Err("command file exceeds 8 MiB budget".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "command file must be UTF-8")?;
    let mut steps = Vec::new();
    let mut captures = HashMap::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if number > MAX_LINES {
            return Err(at(number, "command file exceeds 4096 lines"));
        }
        if line.len() > MAX_LINE_BYTES {
            return Err(at(number, "line exceeds 256 KiB budget"));
        }
        let value: Value = serde_json::from_str(line).map_err(|e| at(number, format!("invalid JSON ({:?}, column {})", e.classify(), e.column())))?;
        let object = value.as_object().ok_or_else(|| at(number, "expected a command object"))?;
        if object.keys().any(|k| !matches!(k.as_str(), "command" | "params" | "saveAs")) {
            return Err(at(number, "only command, params and saveAs fields are allowed"));
        }
        let command =
            object.get("command").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| at(number, "command must be a nonempty string"))?;
        let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
        if !params.is_object() {
            return Err(at(number, "params must be an object"));
        }
        let mut budget = MAX_BYTES;
        template(&params, &captures, false, 0, &mut budget).map_err(|e| at(number, e))?;
        let save_as = match object.get("saveAs") {
            None => None,
            Some(Value::String(name)) if valid_name(name) => Some(name.clone()),
            _ => return Err(at(number, "saveAs must be a name of 1–64 ASCII letters, digits or underscores")),
        };
        if let Some(name) = &save_as {
            if captures.contains_key(name) {
                return Err(at(number, format!("duplicate capture ${name}")));
            }
            if captures.len() >= MAX_CAPTURES {
                return Err(at(number, "capture limit is 256"));
            }
            captures.insert(name.clone(), Value::Null);
        }
        steps.push(Step { line: number, command: command.to_owned(), params, save_as });
    }
    if steps.is_empty() {
        return Err("command file is empty".into());
    }
    Ok(steps)
}

pub(super) fn run(args: &RunArgs, out: &mut dyn Write) -> Result<(), String> {
    // This synchronous entrypoint borrows a writer; reject async hosts before creating or
    // dropping a nested runtime, either of which could panic in that context.
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err("remote command files cannot run inside a Tokio runtime; call the synchronous CLI from a dedicated thread without a Tokio runtime".into());
    }
    let port = args.control.filter(|port| *port != 0).ok_or("--control must be a port from 1 to 65535")?;
    let path = args.file.as_deref().ok_or("remote run needs exactly one <script.jsonl>")?;
    let steps = load(path)?;
    let supplied = std::env::var("PHOTOCRAFT_CONTROL_TOKEN").ok().filter(|value| !value.trim().is_empty());
    let token_file =
        args.control_token_file.clone().or_else(|| std::env::var_os("PHOTOCRAFT_CONTROL_TOKEN_FILE").filter(|value| !value.is_empty()).map(PathBuf::from));
    let token = security::client_token(supplied.as_deref(), token_file.as_deref()).map_err(|e| e.to_string())?;
    let client = BridgeClient::new(format!("127.0.0.1:{port}"), token).map_err(|e| e.to_string())?;
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let mut captures = HashMap::new();
        let mut remaining = MAX_BYTES;
        for step in steps {
            let mut budget = MAX_BYTES;
            let params = template(&step.params, &captures, true, 0, &mut budget).map_err(|e| at(step.line, e))?;
            let result = client.call("engine.execute", json!({"command": step.command, "params": params})).await.map_err(|e| {
                // A server error can echo request data. Keep transport diagnostics, but do not
                // indiscriminately copy server-supplied messages into the local error stream.
                let cause = match e {
                    AutomationError::App(_) => "server rejected command; inspect server diagnostics".to_owned(),
                    other => other.to_string(),
                };
                at(step.line, cause)
            })?;
            if photocraft_engine::actions_cmds::nested_failure(&step.command, &result).is_some() {
                return Err(at(step.line, "command reported a nested failure; inspect server diagnostics"));
            }
            if let Some(name) = step.save_as {
                let size = serde_json::to_vec(&result).map_err(|e| at(step.line, e))?.len();
                remaining = remaining.checked_sub(size).ok_or_else(|| at(step.line, "captured results exceed 8 MiB budget"))?;
                captures.insert(name, result.clone());
            }
            print_json(out, &json!({"command": step.command, "result": result}), JsonStyle::Compact).map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substituted_values_are_budgeted_before_cloning() {
        let captures = HashMap::from([("big".to_owned(), json!("long result"))]);
        assert!(template(&json!({"x":"$big"}), &captures, true, 0, &mut 2).is_err());
    }

    #[test]
    fn captured_subtrees_obey_the_insertion_depth() {
        for value in [json!({"literal": "$customer"}), json!(["$customer"])] {
            let captures = HashMap::from([("saved".to_owned(), value.clone())]);
            let mut budget = MAX_BYTES;
            assert_eq!(template(&json!("$saved"), &captures, true, MAX_DEPTH - 1, &mut budget).unwrap(), value);
            assert!(template(&json!("$saved"), &captures, true, MAX_DEPTH, &mut budget).is_err());
        }
    }

    #[test]
    fn deeply_nested_params_are_refused() {
        let mut params = json!(0);
        for _ in 0..MAX_DEPTH + 1 {
            params = json!([params]);
        }
        assert!(template(&params, &HashMap::new(), false, 0, &mut MAX_BYTES.clone()).is_err());
    }
}
