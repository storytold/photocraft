//! File › Automate › Contact Sheet II and Create Droplet, File › Scripts › Statistics, Browse
//! and Script Events Manager, plus the session state the File-menu commands share
//! ([`FileMenuState`]).
//!
//! "Scripts" are PhotoCraft action scripts: JSON (`[[id, params], …]`, `{"steps": …}` or a
//! droplet) or a plain-text list with one `command.id {json params}` per line (`#` comments).
//! Droplets are JSON files (`.pcdroplet`) holding an action plus batch options; `photocraft-cli
//! droplet <file> <inputs…>` runs one, and on Unix a `.command` shim makes it
//! double-clickable / drop-target-able from the shell.
//!
//! Script events (Start Application, New / Open / Save / Close Document, Print, Export) are
//! bound in Preferences (`scriptEvents`) and fired by [`fire_event`]: the engine fires New,
//! Close, Print and Export itself; the app shell fires Start, Open and Save.

use crate::actions_cmds::ActionsExt;
use crate::preset_store::PresetStoreExt;
use photocraft_doc::{Layer, LayerContent};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::file_cmds::{file_name, import, join, list_images, list_images_tree, read_file, stem, write_file};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Photoshop ExtendScript / UXP script suffixes — not PhotoCraft JSON or command lists.
fn extendscript_ext(path: &str) -> Option<&str> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let dot = name.rfind('.')?;
    let ext = &name[dot + 1..];
    let lower = ext.to_ascii_lowercase();
    matches!(lower.as_str(), "js" | "jsx" | "jsxbin" | "psjs" | "ccjs").then_some(ext)
}

fn reject_extendscript(cmd: &str, path: &str) -> Result<()> {
    match extendscript_ext(path) {
        Some(_) => Err(bad(cmd, "PhotoCraft scripts are JSON or command lists (one command.id {json} per line), not Photoshop ExtendScript (.js/.jsx)")),
        None => Ok(()),
    }
}

pub use crate::state::FileMenuState;

// ---------- scripts ----------

/// Parses an action script: JSON (array of steps, `{"steps"|"action": …}`, a droplet) or one
/// `command.id {json}` per line.
pub fn parse_script(text: &str) -> Result<Vec<(String, Value)>> {
    let cmd = "file.scripts.browse";
    let t = text.trim_start_matches('\u{feff}').trim();
    if t.starts_with('[') || t.starts_with('{') {
        let v: Value = serde_json::from_str(t).map_err(|e| bad(cmd, format!("script JSON: {e}")))?;
        return parse_action(&v, cmd);
    }
    let mut out = Vec::new();
    for (n, line) in t.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let (id, rest) = line.split_once(char::is_whitespace).map_or((line, ""), |(a, b)| (a, b.trim()));
        let params = if rest.is_empty() { json!({}) } else { serde_json::from_str(rest).map_err(|e| bad(cmd, format!("line {}: {e}", n + 1)))? };
        out.push((id.to_string(), params));
    }
    Ok(out)
}

/// The steps of a JSON action: an array of steps (`[id, params]`, `{"command", "params"}` or a
/// bare id), `{"steps"|"action": …}` (a recorded action) or a droplet. `cmd` names the caller in
/// errors.
pub fn parse_action(v: &Value, cmd: &str) -> Result<Vec<(String, Value)>> {
    let steps = match v {
        Value::Array(_) => v,
        Value::Object(o) => {
            o.get("steps").or_else(|| o.get("action").and_then(|a| a.get("steps").or(Some(a)))).ok_or_else(|| bad(cmd, "an action object needs \"steps\""))?
        }
        _ => return Err(bad(cmd, "an action must be an array of steps or an object")),
    };
    parse_steps(steps, cmd)
}

fn parse_steps(v: &Value, cmd: &str) -> Result<Vec<(String, Value)>> {
    let arr = v.as_array().ok_or_else(|| bad(cmd, "steps must be an array"))?;
    arr.iter()
        .map(|st| match st {
            Value::Array(a) if !a.is_empty() => {
                Ok((a[0].as_str().ok_or_else(|| bad(cmd, "step id must be a string"))?.to_string(), a.get(1).cloned().unwrap_or(json!({}))))
            }
            Value::Object(o) => {
                let id = o.get("command").or_else(|| o.get("id")).and_then(Value::as_str).ok_or_else(|| bad(cmd, "step needs \"command\""))?;
                Ok((id.to_string(), o.get("params").cloned().unwrap_or(json!({}))))
            }
            Value::String(id) => Ok((id.clone(), json!({}))),
            _ => Err(bad(cmd, "each step is [id, params], {\"command\", \"params\"} or an id")),
        })
        .collect()
}

fn check_known(steps: &[(String, Value)], cmd: &str) -> Result<()> {
    match steps.iter().find(|(id, _)| crate::commands::find(id).is_none()) {
        Some((id, _)) => Err(bad(cmd, format!("unknown command `{id}` in the script"))),
        None => Ok(()),
    }
}

/// Scripts nested in scripts, and steps one top-level script (with everything it calls) may run:
/// a script can name others, so a graph that calls each of the previous ones twice is exponential.
const MAX_SCRIPT_DEPTH: u8 = 16;
pub(crate) const MAX_SCRIPT_STEPS: u64 = crate::actions_cmds::MAX_PLAYBACK_STEPS;

/// Direct Batch file errors remain script results; called action failures stop later script steps.
fn run_steps(s: &mut Session, steps: &[(String, Value)]) -> Vec<Value> {
    let mut results = Vec::new();
    for (id, params) in steps {
        s.recording.script_steps = s.recording.script_steps.saturating_add(1);
        if s.recording.script_steps > MAX_SCRIPT_STEPS {
            results.push(json!({"command": id, "error": format!("the script ran more than {MAX_SCRIPT_STEPS} steps in all (scripts calling scripts)")}));
            break;
        }
        match s.execute(id, params.clone()) {
            Ok(r) => {
                // Direct Batch calls report file errors successfully to scripts, but running out of
                // script steps, a cancel or a failed error log stops it. Called actions still fail it.
                let failure = if id == "file.automate.batch" { crate::actions_cmds::batch_stop(&r) } else { crate::actions_cmds::nested_failure(id, &r) };
                match failure {
                    Some(error) => {
                        results.push(json!({"command": id, "result": r, "error": error}));
                        break;
                    }
                    None => results.push(json!({"command": id, "result": r})),
                }
            }
            Err(e) => {
                results.push(json!({"command": id, "error": e.to_string()}));
                break;
            }
        }
    }
    results
}

/// File › Scripts › Browse…: run a script file on the session.
fn browse(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.scripts.browse";
    if let Some(path) = p.get("path").and_then(Value::as_str) {
        reject_extendscript(cmd, path)?;
    }
    if let Some(name) = p.get("name").and_then(Value::as_str) {
        reject_extendscript(cmd, name)?;
    }
    let steps = match (p.get("path").and_then(Value::as_str), p.get("script").and_then(Value::as_str), p.get("steps")) {
        (Some(path), _, _) => parse_script(&String::from_utf8_lossy(&read_file(path)?))?,
        (_, Some(text), _) => parse_script(text)?,
        (_, _, Some(v)) => parse_steps(v, cmd)?,
        _ => return Err(bad(cmd, "give \"path\" (a script file), \"script\" (its text) or \"steps\"")),
    };
    check_known(&steps, cmd)?;
    if s.recording.script_depth >= MAX_SCRIPT_DEPTH {
        return Err(bad(cmd, format!("scripts are nested more than {MAX_SCRIPT_DEPTH} deep")));
    }
    if s.recording.script_depth == 0 {
        s.recording.script_steps = 0;
    }
    s.recording.script_depth += 1;
    let results = run_steps(s, &steps);
    s.recording.script_depth -= 1;
    let ok = results.iter().all(|r| r.get("error").is_none());
    Ok(json!({"steps": steps.len(), "ok": ok, "results": results}))
}

// ---------- Script Events Manager ----------

/// Events a script can be bound to (Photoshop's Script Events Manager list).
pub const EVENTS: [(&str, &str); 8] = [
    ("startApplication", "Start Application"),
    ("newDocument", "New Document"),
    ("openDocument", "Open Document"),
    ("saveDocument", "Save Document"),
    ("closeDocument", "Close Document"),
    ("print", "Print Document"),
    ("export", "Export Document"),
    ("everything", "Everything"),
];

pub use crate::prefs::{ScriptBinding, ScriptEvents};

fn script_events(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.scripts.scriptEventsManager";
    let mut ev = s.prefs().script_events.clone();
    if let Some(b) = p.get("enabled").and_then(Value::as_bool) {
        ev.enabled = b;
    }
    if p.get("removeAll").and_then(Value::as_bool) == Some(true) {
        ev.bindings.clear();
    }
    if let Some(i) = crate::commands::int(p, "remove").filter(|v| *v >= 0) {
        if (i as usize) >= ev.bindings.len() {
            return Err(bad(cmd, format!("no binding {i}")));
        }
        ev.bindings.remove(i as usize);
    }
    // The dialog's flat form: "event" + "script" at the top level means add.
    let flat = p
        .get("event")
        .and_then(Value::as_str)
        .filter(|_| p.get("script").and_then(Value::as_str).is_some_and(|v| !v.is_empty()))
        .map(|e| json!({"event": e, "script": p["script"], "name": p.get("name").cloned().unwrap_or(Value::Null)}));
    if let Some(a) = p.get("add").cloned().or(flat).as_ref() {
        let event = a.get("event").and_then(Value::as_str).ok_or_else(|| bad(cmd, "add needs \"event\""))?;
        if !EVENTS.iter().any(|(id, _)| *id == event) {
            return Err(bad(cmd, format!("unknown event `{event}` ({})", EVENTS.map(|e| e.0).join("|"))));
        }
        let script = a.get("script").and_then(Value::as_str).map(str::to_string);
        let steps = a.get("steps").cloned();
        if script.is_none() && steps.is_none() {
            return Err(bad(cmd, "add needs \"script\" (a file) or \"steps\" (an action)"));
        }
        if let Some(st) = &steps {
            check_known(&parse_steps(st, cmd)?, cmd)?;
        }
        let name = a
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| script.as_deref().map(file_name).unwrap_or_else(|| "Action".into()));
        ev.bindings.push(ScriptBinding { event: event.into(), script, steps, name });
        if a.get("enable").and_then(Value::as_bool).unwrap_or(true) {
            ev.enabled = true;
        }
    }
    let changed = ev != s.prefs().script_events;
    if changed {
        s.edit_prefs(|pr| pr.script_events = ev.clone());
    }
    let events: Vec<Value> = EVENTS.iter().map(|(id, label)| json!({"id": id, "label": label})).collect();
    Ok(json!({"enabled": ev.enabled, "bindings": ev.bindings, "events": events}))
}

/// Runs the scripts bound to `event` (when Script Events are enabled). Returns how many ran.
pub fn fire_event(s: &mut Session, event: &str) -> usize {
    if s.file_menu.firing {
        return 0;
    }
    let ev = &s.prefs().script_events;
    if !ev.enabled {
        return 0;
    }
    let jobs: Vec<ScriptBinding> = ev.bindings.iter().filter(|b| b.event == event || b.event == "everything").cloned().collect();
    if jobs.is_empty() {
        return 0;
    }
    s.file_menu.firing = true;
    for b in &jobs {
        let steps = match (&b.steps, &b.script) {
            (Some(st), _) => parse_steps(st, "scriptEvent"),
            (None, Some(path)) => read_file(path).and_then(|bytes| parse_script(&String::from_utf8_lossy(&bytes))),
            _ => Ok(Vec::new()),
        };
        let entry = match steps {
            Ok(steps) => json!({"event": event, "binding": b.name, "results": run_steps(s, &steps)}),
            Err(e) => json!({"event": event, "binding": b.name, "error": e.to_string()}),
        };
        s.file_menu.event_log.push(entry);
    }
    let n = s.file_menu.event_log.len();
    if n > 32 {
        s.file_menu.event_log.drain(..n - 32);
    }
    s.file_menu.firing = false;
    jobs.len()
}

/// Called by `Session::execute` after every successful command: keeps layer-based slices on
/// their layers and fires the engine-side script events.
pub(crate) fn after_command(s: &mut Session, id: &str) {
    crate::slice_cmds::refresh(s);
    let event = match id {
        "file.new" => "newDocument",
        "file.close" | "file.closeAll" | "file.closeOthers" => "closeDocument",
        _ => return,
    };
    fire_event(s, event);
}

/// App shell hooks for the events only it sees.
pub fn document_opened(s: &mut Session) {
    fire_event(s, "openDocument");
}

/// After the shell saved document `doc`: Save Document scripts and Image Assets.
/// The script runs first and may close or reorder tabs, so asset generation resolves the saved
/// document's identity again: a script that closed a different tab must not skip its assets, and
/// one that closed the saved document must not generate a replacement tab's (#11121).
pub fn document_saved(s: &mut Session, doc: photocraft_doc::DocId) -> Option<Value> {
    fire_event(s, "saveDocument");
    let index = s.documents().iter().position(|st| st.doc.id == doc)?;
    crate::web_cmds::on_saved(s, index)
}

// ---------- Create Droplet ----------

/// Resolve the enabled steps of the saved action chosen for a droplet: by name, refused when a
/// given `set` does not hold it, else by name or index.
pub(crate) fn named_action_steps(s: &Session, p: &Value, cmd: &str) -> Result<Vec<(String, Value)>> {
    let index = match p.get("set").filter(|v| !v.is_null()) {
        Some(set) => {
            let set = set.as_str().ok_or_else(|| bad(cmd, "\"set\" must be a string"))?;
            let name = p.get("action").and_then(Value::as_str).ok_or_else(|| bad(cmd, "a set needs a named action"))?;
            s.actions()
                .list
                .actions()
                .iter()
                .position(|a| a.name == name && a.set.as_deref() == Some(set))
                .ok_or_else(|| bad(cmd, format!("no action named `{name}` in set `{set}`")))?
        }
        None => crate::actions_cmds::resolve(s.actions().list.actions(), p, cmd)?,
    };
    let action = s.actions().list.action(index).ok_or_else(|| bad(cmd, "the selected action no longer exists"))?;
    Ok(action.enabled_steps().map(|(_, step)| (step.command.clone(), step.params.clone())).collect())
}

/// The Play options of Create Droplet, in the dialog's order: Override Action "Open" Commands,
/// Include All Subfolders, Suppress File Open Options Dialogs, Suppress Color Profile Warnings.
const DROPLET_PLAY_OPTIONS: [&str; 4] = ["overrideOpen", "includeSubfolders", "suppressOpenOptions", "suppressWarnings"];

/// Save the chosen action's enabled steps and dependencies as a runnable droplet.
fn create_droplet(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.automate.createDroplet";
    let path = p.get("path").and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| bad(cmd, "missing \"path\" (where to save the droplet)"))?;
    let mut steps = match p.get("steps").or_else(|| p.get("action").filter(|v| v.is_array())) {
        Some(steps) => parse_steps(steps, cmd)?,
        None => named_action_steps(s, p, cmd)?,
    };
    // As in batch, the droplet's output folder replaces recorded saves, which are kept to mark the
    // result, and view steps don't apply.
    steps.retain(|(id, _)| crate::file_cmds::batch_keeps_step(id));
    let commands: Vec<_> = steps.iter().filter(|(id, _)| !crate::file_cmds::batch_save_point(id)).cloned().collect();
    check_known(&commands, cmd)?;
    // Play Action steps need their actions when the droplet runs on its own (#2794). Not under
    // `actions`: the CLI reads that key as an action set.
    let called = crate::actions_cmds::called_actions(s.actions().list.actions(), &mut steps);
    let path = if path.ends_with(".pcdroplet") || path.ends_with(".json") { path.to_string() } else { format!("{path}.pcdroplet") };
    let name = p.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&path));
    let steps_json: Vec<Value> = steps.iter().map(|(id, pr)| json!([id, pr])).collect();
    let mut options = serde_json::Map::new();
    // The Create Droplet Destination (#13711): the form's choice rides with the droplet and
    // `run_droplet` acts on it. A call that names no destination stores no key, so droplets
    // written by earlier builds, and by API calls without the choice, keep running as Folder.
    let dest_choice = match p.get("destination").filter(|v| !v.is_null()) {
        None => None,
        Some(Value::String(value)) if matches!(value.as_str(), "none" | "folder" | "saveAndClose") => {
            options.insert("destination".into(), json!(value.as_str()));
            Some(value.as_str())
        }
        Some(_) => return Err(bad(cmd, "\"destination\" must be \"none\", \"folder\" or \"saveAndClose\"")),
    };
    // `overrideSaveAs` is the form's Override Action "Save As" Commands tick, stored as the issue
    // asks. The hosted form answers the untouched box with false, so only a ticked box is stored:
    // an off box must mean exactly today's run, which stored no such key. `run_droplet` does not
    // act on either state (not measured for droplet runs, see #13711).
    if p.get("overrideSaveAs").and_then(Value::as_bool) == Some(true) {
        options.insert("overrideSaveAs".into(), json!(true));
    }
    for k in ["output", "format", "quality", "suffix"] {
        // "output" rides only with a folder droplet (#13711 review): neither a none nor a
        // saveAndClose run ever reads a folder, and a stored destination of none or saveAndClose
        // only exists in droplets written by this code, so leaving the key out keeps behaviour.
        if k == "output" && matches!(dest_choice, Some("none" | "saveAndClose")) {
            continue;
        }
        if let Some(v) = p.get(k) {
            options.insert(k.into(), v.clone());
        }
    }
    // The dialog's Play checkboxes are saved with the droplet, which hands them to each run.
    for k in DROPLET_PLAY_OPTIONS {
        match p.get(k) {
            None | Some(Value::Null) => {}
            Some(Value::Bool(on)) => {
                options.insert(k.into(), json!(on));
            }
            Some(_) => return Err(bad(cmd, format!("\"{k}\" must be true or false"))),
        }
    }
    let mut droplet = json!({"photocraftDroplet": 1, "name": name, "action": {"name": name, "steps": steps_json}, "options": options});
    if !called.is_empty() {
        droplet["calledActions"] = json!(called);
    }
    let text = serde_json::to_string_pretty(&droplet).unwrap_or_default();
    write_file(&path, text.as_bytes())?;
    // A none droplet writes no shim (#13711 review): its shim calls `photocraft-cli droplet`, and
    // a headless run of a none droplet is always refused - a launcher built from it could only
    // ever print that refusal.
    let shim = dest_choice != Some("none") && p.get("shim").and_then(Value::as_bool).unwrap_or(cfg!(unix));
    let shim_path = if shim { Some(write_shim(&path)?) } else { None };
    Ok(json!({"path": path, "shim": shim_path, "steps": steps.len()}))
}

/// A tiny shell script next to the droplet: drop files on it (or pass them) to run the droplet.
#[cfg(not(target_arch = "wasm32"))]
fn write_shim(droplet: &str) -> Result<String> {
    let abs = std::fs::canonicalize(droplet).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| droplet.to_string());
    let base = abs.trim_end_matches(".pcdroplet").trim_end_matches(".json");
    let shim = format!("{base}.command");
    // Single quotes keep shell expansions inert; embedded quotes need to end the
    // quoted segment, emit a literal quote, then resume it.
    let quoted_abs = format!("'{}'", abs.replace('\'', "'\\''"));
    let body = format!(
        r#"#!/bin/sh
# PhotoCraft droplet: runs the action on the files given (or dropped).
if [ -n "${{PHOTOCRAFT_CLI:-}}" ]; then
  cli="$PHOTOCRAFT_CLI"
elif [ -x /Applications/PhotoCraft.app/Contents/MacOS/photocraft-cli ]; then
  cli=/Applications/PhotoCraft.app/Contents/MacOS/photocraft-cli
elif [ -x "$HOME/Applications/PhotoCraft.app/Contents/MacOS/photocraft-cli" ]; then
  cli="$HOME/Applications/PhotoCraft.app/Contents/MacOS/photocraft-cli"
else
  cli=photocraft-cli
fi
exec "$cli" droplet {quoted_abs} "$@"
"#
    );
    crate::file_cmds::write_file(&shim, body.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The shim only has to run as the user who created it; 0o755 would also let every other
        // local account read and execute it.
        let _ = std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o700));
    }
    Ok(shim)
}
#[cfg(target_arch = "wasm32")]
fn write_shim(_: &str) -> Result<String> {
    Err(EngineError::Other("no file system on the web".into()))
}

/// Runs a droplet file on inputs (files and/or folders).
fn run_droplet(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.automate.runDroplet";
    // Checked before the droplet file is read, so a droplet that runs itself stops at the limit.
    crate::file_cmds::check_batch_depth(s, cmd)?;
    let path = p.get("droplet").or_else(|| p.get("path")).and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing \"droplet\""))?;
    let v: Value = serde_json::from_slice(&read_file(path)?).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    if v.get("photocraftDroplet").is_none() {
        return Err(bad(cmd, format!("{path} is not a PhotoCraft droplet")));
    }
    let steps = v.get("action").and_then(|a| a.get("steps")).cloned().ok_or_else(|| bad(cmd, "droplet has no action"))?;
    let opts = v.get("options").cloned().unwrap_or(json!({}));
    // Where the run writes (the Create Droplet Destination dropdown, #13711), stored with the
    // droplet. A droplet written before the change names no destination: it runs as Folder, which
    // is exactly what it does today.
    let destination = match opts.get("destination") {
        None | Some(Value::Null) => "folder",
        Some(Value::String(value)) if matches!(value.as_str(), "none" | "folder" | "saveAndClose") => value.as_str(),
        Some(_) => return Err(bad(cmd, "the droplet's \"destination\" must be \"none\", \"folder\" or \"saveAndClose\"")),
    };
    // A droplet saved before Include All Subfolders existed has no key and lists a folder's own images.
    let include_subfolders = match opts.get("includeSubfolders") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(on)) => *on,
        Some(_) => return Err(bad(cmd, format!("{path}: \"includeSubfolders\" must be true or false"))),
    };
    let mut output = p.get("output").or_else(|| opts.get("output")).and_then(Value::as_str).map(str::to_string);
    // Without a chosen output the results go next to the first input.
    let default_output = |first: &str| {
        let first = std::path::Path::new(first);
        join(&first.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(), "droplet-output")
    };
    let mut inputs: Vec<String> = Vec::new();
    let raw: Vec<String> = match p.get("input").or_else(|| p.get("files")) {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(one)) => vec![one.clone()],
        _ => return Err(bad(cmd, "missing \"input\" (files or folders)")),
    };
    for i in raw {
        if std::path::Path::new(&i).is_dir() {
            // The subfolder walk doesn't go into the output folder below a dropped one, so a rerun
            // doesn't take its results as inputs: the chosen one, else the first input's default,
            // else this folder's own `droplet-output`, which then becomes the output. A dropped
            // folder is taken as given, as without the walk (and as Batch takes an input folder
            // that is also its output).
            let skip = output.clone().or_else(|| inputs.first().map(|first| default_output(first))).unwrap_or_else(|| join(&i, "droplet-output"));
            let found = folder_images(&i, include_subfolders, &skip, cmd)?;
            if include_subfolders && output.is_none() && !found.is_empty() {
                output = Some(skip);
            }
            inputs.extend(found)
        } else if crate::file_cmds::openable_name(&i) {
            inputs.push(i)
        }
    }
    if inputs.is_empty() {
        return Err(bad(cmd, "no input images"));
    }
    // Destination None never reaches Batch (#13711): it keeps the opened documents here and writes
    // nothing, and a headless run of it is refused - see `run_droplet_open`.
    if destination == "none" {
        return run_droplet_open(s, cmd, path, &steps, &inputs, v.get("calledActions"));
    }
    let mut bp = json!({"steps": steps, "input": inputs, "format": opts.get("format").cloned().unwrap_or(json!("same"))});
    let output = if destination == "saveAndClose" {
        // Batch's Save and Close (#13659) writes each source over itself in its own format and
        // takes no output folder: neither the stored `output` nor a call-site one is passed on.
        bp["destination"] = json!("saveAndClose");
        Value::Null
    } else {
        // The folder the call or the droplet chose, else the one the Include All Subfolders walk
        // settled on above, else next to the first input.
        let output = output.unwrap_or_else(|| default_output(&inputs[0]));
        bp["output"] = json!(&output);
        json!(output)
    };
    if let Some(name) = v.get("action").and_then(|action| action.get("name")).or_else(|| v.get("name")) {
        bp["action"] = name.clone();
    }
    // Override Open is saved with the droplet: a recorded Open then reads each dropped input, and the
    // two Suppress options open the inputs as Batch does with them; Include All Subfolders was applied
    // above, where the dropped folders became files. The stored Override "Save As" tick is not passed
    // on: its effect during a droplet run is not measured (#13711), so the run keeps today's Batch
    // behaviour exactly.
    for key in ["quality", "overrideOpen", "suppressOpenOptions", "suppressWarnings"] {
        if let Some(value) = opts.get(key) {
            bp[key] = value.clone();
        }
    }
    if let Some(called) = v.get("calledActions") {
        bp["calledActions"] = called.clone();
    }
    let r = s.execute("file.automate.batch", bp)?;
    Ok(json!({"droplet": path, "output": output, "files": r["files"], "errors": r["errors"]}))
}

/// Destination None of the Create Droplet dialog (#13711), run in the app: each input opens as a
/// document of this session, the action's steps play on it, and nothing is written - the tab stays
/// open and changed, its last History state the action's step. Only the running app can keep those
/// tabs: the desktop and web shells attach a preset store to their session once it loads
/// (`crate::preset_store`), the headless sessions (`photocraft-cli droplet`, MCP, tests) never do,
/// and there the run is refused with an error naming the app instead of opening documents nobody
/// will see. (The shells attach the store a few frames into startup; a droplet drop or a menu run
/// is a user action after that, and an early refusal says so rather than guessing.)
fn run_droplet_open(s: &mut Session, cmd: &str, path: &str, steps: &Value, inputs: &[String], called: Option<&Value>) -> Result<Value> {
    if s.preset_store().is_none() {
        return Err(EngineError::Other(format!(
            "{cmd}: this droplet's Destination is None, which leaves the opened documents in the app: run it in the app, or give the droplet a Folder or Save and Close"
        )));
    }
    // The steps are parsed and filtered first, before any of the caller's state changes: a stored
    // droplet whose steps do not parse is refused here, and the run has installed nothing and
    // raised nothing yet (#13711 review, automate_cmds.rs:573). The steps are exactly the ones
    // Batch's folder run keeps (its retained-steps rule, quickExport included): view steps go,
    // recorded saves are skipped - they mark a destination None has none of, write nothing, and
    // leave the document changed, as Batch with no destination does.
    let steps: Vec<(String, Value)> =
        parse_steps(steps, cmd)?.into_iter().filter(|(id, _)| crate::file_cmds::batch_keeps_step(id) && id != "file.export.quickExport").collect();
    // The droplet plays from the actions it captured (#2794's rule): install them for this run, as
    // the folder path forwards `calledActions` to Batch, so a Play Action step cannot fall back to
    // - or play a wrong same-named action of - the app's live list.
    let embedded = match called {
        None | Some(Value::Null) => None,
        Some(v) => {
            let actions: Vec<crate::actions_cmds::Action> = serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("\"calledActions\": {e}")))?;
            if actions.len() > 4096 {
                return Err(bad(cmd, "\"calledActions\" holds more than 4096 actions"));
            }
            // In the droplet's order, so its calls by index keep their targets (Batch's own rule).
            Some(crate::actions_cmds::ActionList::from_file(crate::actions_cmds::ActionsFile::new(actions, Vec::new())))
        }
    };
    let previous_called = std::mem::replace(&mut s.actions_mut().called, embedded);
    // The folder run nests through Batch's own scratch sessions, each raised `batch_depth`; this
    // run plays in the caller's session, so it raises the depth itself and hands it back - a
    // droplet that runs droplets stops at `MAX_BATCH_DEPTH` with the shared error text instead of
    // opening one document per level until the stack or memory runs out (#13711 review HIGH-1).
    let previous_depth = s.recording.batch_depth;
    s.recording.batch_depth = previous_depth.saturating_add(1);
    let mut errors = Vec::new();
    for input in inputs {
        let ran = (|| -> Result<()> {
            // The shared open core, as File > Open and Batch's scratch run use it: import options,
            // Raw Defaults resolution, the Color Settings policy and the read-only source mark -
            // not a bare import (#13711 review MEDIUM-3).
            crate::file_cmds::open_bytes_as(s, &file_name(input), &read_file(input)?, None, Some(input.clone()))?;
            for (id, params) in &steps {
                if crate::file_cmds::batch_save_point(id) {
                    continue;
                }
                // A droplet run inside a script charges each step to the script's total, as Batch does.
                if s.recording.script_depth > 0 {
                    if s.recording.script_steps >= MAX_SCRIPT_STEPS {
                        return Err(EngineError::Other(format!("the script ran more than {MAX_SCRIPT_STEPS} steps in all (scripts calling scripts)")));
                    }
                    s.recording.script_steps += 1;
                }
                let params = crate::actions_cmds::playback_params(s, id, params)?;
                let response = s.execute(id, params)?;
                // A nested batch, droplet or played action that hit the nesting or shared playback
                // limit fails this input too, instead of letting it pass as processed - Batch's own
                // step-loop promotion, mirrored so the stop reaches the outermost answer.
                if let Some(limit) = crate::actions_cmds::limit_stop(&response) {
                    return Err(EngineError::BatchLimit(limit));
                }
                if let Some(error) = crate::actions_cmds::nested_failure(id, &response) {
                    return Err(EngineError::Other(error));
                }
            }
            Ok(())
        })();
        if let Err(error) = ran {
            let mut entry = json!({"file": input, "error": error.to_string()});
            // The mark Batch's FileFailure puts on a limit stop, so the next enclosing run (and
            // its parent through `limit_stop`) promotes it instead of swallowing it.
            if matches!(error, EngineError::BatchLimit(_)) {
                entry["limit"] = json!(true);
            }
            errors.push(entry);
        }
    }
    s.recording.batch_depth = previous_depth;
    s.actions_mut().called = previous_called;
    Ok(json!({"droplet": path, "output": null, "files": [], "errors": errors}))
}

/// The images a dropped folder contributes: its own, sorted by name, or with Include All
/// Subfolders also every subfolder's, depth first as Batch walks them, without going into the
/// subfolder `skip` (the output). Results land flat in the output folder, so a later image
/// replaces an earlier one of the same name.
fn folder_images(dir: &str, include_subfolders: bool, skip: &str, cmd: &str) -> Result<Vec<String>> {
    if !include_subfolders {
        return list_images(dir);
    }
    Ok(crate::file_cmds::list_images_tree(dir, skip, cmd)?.into_iter().map(|input| input.path).collect())
}

// ---------- Statistics ----------

fn statistics(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.scripts.statistics";
    let mode = p.get("mode").or_else(|| p.get("stackMode")).and_then(Value::as_str).unwrap_or("median");
    let sm = photocraft_doc::StackMode::ALL
        .into_iter()
        .find(|m| m.id().eq_ignore_ascii_case(mode))
        .ok_or_else(|| bad(cmd, format!("unknown stack mode `{mode}`")))?;
    let input = p.get("input").or_else(|| p.get("paths")).cloned().ok_or_else(|| bad(cmd, "missing \"input\" (files or a folder)"))?;
    let align = p.get("align").and_then(Value::as_bool).unwrap_or(false);
    let r = s.execute("file.scripts.loadFilesIntoStack", json!({"paths": input, "align": align, "createSmartObject": true}))?;
    let n = r["layers"].as_u64().unwrap_or(0);
    let id = s.active().and_then(|d| d.doc.layers.first().map(|l| l.id)).ok_or(EngineError::NoDocument)?;
    s.select_layer(id)?;
    s.execute(&format!("layer.smartObjects.stackMode.{}", sm.id()), json!({"layer": id.0}))?;
    Ok(json!({"document": s.active_index(), "layer": id.0, "images": n, "mode": sm.id()}))
}

// ---------- Contact Sheet II ----------

fn units_px(v: f64, unit: &str, res: f64) -> f64 {
    match unit {
        "cm" => v * res / 2.54,
        "mm" => v * res / 25.4,
        "px" | "pixels" => v,
        _ => v * res,
    }
}

/// Contact Sheet II's images, one list per run of sheets. A folder `input` lists its own images by
/// name and, with `includeSubfolders` (default on), then each subfolder's depth first, each level
/// by name, bounded as Batch's walk is ([`list_images_tree`]: depth and file limits, a folder
/// reached twice through a link walked once). `groupByFolder` (default off) makes each folder
/// that has images its own list, so it starts on a new sheet. An array `input` is one list in its
/// own order, whatever the two flags say.
fn contact_sheet_sources(p: &Value, cmd: &str) -> Result<Vec<Vec<String>>> {
    let flag = |key: &str, default: bool| match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(bad(cmd, format!("\"{key}\" must be true or false"))),
    };
    let subfolders = flag("includeSubfolders", true)?;
    let by_folder = flag("groupByFolder", false)?;
    match p.get("input").or_else(|| p.get("files")) {
        Some(Value::Array(a)) => Ok(vec![a.iter().filter_map(Value::as_str).map(str::to_string).collect()]),
        Some(Value::String(dir)) if subfolders => {
            let tree = list_images_tree(dir, "", cmd)?;
            let paths = |run: &[crate::file_cmds::BatchInput]| run.iter().map(|i| i.path.clone()).collect::<Vec<_>>();
            // The walk lists each folder's files together, so a folder is one run of `rel_dir`.
            Ok(if by_folder { tree.chunk_by(|a, b| a.rel_dir == b.rel_dir).map(paths).collect() } else { vec![paths(&tree)] })
        }
        Some(Value::String(dir)) => Ok(vec![list_images(dir)?]),
        _ => Err(bad(cmd, "missing \"input\" (a folder or an array of files)")),
    }
}

fn contact_sheet(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.automate.contactSheetII";
    let groups = contact_sheet_sources(p, cmd)?;
    if groups.iter().all(Vec::is_empty) {
        return Err(bad(cmd, "no images to place"));
    }
    let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(d);
    let unit = p.get("units").and_then(Value::as_str).unwrap_or("inches");
    let res = f("resolution", 300.0).clamp(1.0, 10_000.0);
    let pw = units_px(f("width", 8.0), unit, res).round().clamp(16.0, 30_000.0) as u32;
    let ph = units_px(f("height", 10.0), unit, res).round().clamp(16.0, 30_000.0) as u32;
    let cols = f("columns", 5.0).round().clamp(1.0, 100.0) as u32;
    let rows = f("rows", 6.0).round().clamp(1.0, 100.0) as u32;
    let auto = p.get("autoSpacing").and_then(Value::as_bool).unwrap_or(true);
    let gap = (f64::from(pw.min(ph)) * 0.014).max(2.0);
    let hs = if auto { gap } else { p.get("horizontal").and_then(Value::as_f64).map_or(gap, |v| units_px(v, unit, res)) };
    let vs = if auto { gap } else { p.get("vertical").and_then(Value::as_f64).map_or(gap, |v| units_px(v, unit, res)) };
    let across = p.get("placeAcrossFirst").and_then(Value::as_bool).unwrap_or(true);
    let rotate = p.get("rotateForBestFit").and_then(Value::as_bool).unwrap_or(false);
    let caption = p.get("caption").and_then(Value::as_bool).unwrap_or(true);
    let font = p.get("font").and_then(Value::as_str).unwrap_or(photocraft_text::fonts::DEFAULT_FAMILY).to_string();
    let font_pt = f("fontSize", 12.0);
    let flatten = p.get("flatten").and_then(Value::as_bool).unwrap_or(false);
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("rgb").to_string();
    let depth = crate::commands::int(p, "depth").unwrap_or(8);
    let caption_px = if caption { font_pt * res / 72.0 * 1.5 } else { 0.0 };
    let cell_w = ((f64::from(pw) - f64::from(cols + 1) * hs) / f64::from(cols)).max(1.0);
    let cell_h = ((f64::from(ph) - f64::from(rows + 1) * vs) / f64::from(rows)).max(1.0);
    let box_h = (cell_h - caption_px).max(1.0);
    let per_page = (cols * rows) as usize;
    let mut docs = Vec::new();
    let mut placed = 0usize;
    let mut errors = Vec::new();
    // A full sheet continues on the next one; with `groupByFolder` each folder also starts one.
    for (page, chunk) in groups.iter().flat_map(|g| g.chunks(per_page)).enumerate() {
        let name = format!("ContactSheet-{:03}", page + 1);
        s.execute("file.new", json!({"width": pw, "height": ph, "mode": mode, "depth": depth, "background": "white", "name": name}))?;
        let idx = s.active_index().ok_or(EngineError::NoDocument)?;
        if let Some(st) = s.active_mut() {
            std::sync::Arc::make_mut(st.doc_mut()).resolution_dpi = res as f32;
        }
        let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
        for (k, path) in chunk.iter().enumerate() {
            let k = k as u32;
            let (c, r) = if across { (k % cols, k / cols) } else { (k / rows, k % rows) };
            let x0 = hs + f64::from(c) * (cell_w + hs);
            let y0 = vs + f64::from(r) * (cell_h + vs);
            let thumb = (|| -> Result<photocraft_doc::Surface> {
                let bytes = read_file(path)?;
                let src = import(&file_name(path), &bytes)?;
                let mut t = Session::new();
                t.add_document(src, None);
                let (w, h) = t.active().map(|d| (f64::from(d.doc.size.width), f64::from(d.doc.size.height))).unwrap_or((1.0, 1.0));
                let fit = |w: f64, h: f64| (cell_w / w).min(box_h / h);
                if rotate && fit(h, w) > fit(w, h) * 1.001 {
                    t.execute("image.imageRotation.90cw", json!({}))?;
                }
                let (w, h) = t.active().map(|d| (f64::from(d.doc.size.width), f64::from(d.doc.size.height))).unwrap_or((1.0, 1.0));
                let k = fit(w, h);
                let (tw, th) = ((w * k).round().max(1.0), (h * k).round().max(1.0));
                t.execute("image.imageSize", json!({"width": tw, "height": th, "resample": "bicubic"}))?;
                let d = &t.active().ok_or(EngineError::NoDocument)?.doc;
                let buf = photocraft_compose::flatten(d);
                let ox = (x0 + (cell_w - tw) / 2.0).round() as i32;
                let oy = (y0 + (box_h - th) / 2.0).round() as i32;
                let n = fmt.channels();
                let mut data = vec![0.0f32; buf.px.len() * n];
                for (px, out) in buf.px.iter().zip(data.chunks_exact_mut(n)) {
                    photocraft_raster::from_rgba_into(&fmt, *px, out);
                }
                let mut surf = photocraft_doc::Surface::new(fmt);
                surf.write_region(photocraft_geom::Rect::new(ox, oy, ox + tw as i32, oy + th as i32), &data);
                Ok(surf)
            })();
            let surf = match thumb {
                Ok(s) => s,
                Err(e) => {
                    errors.push(json!({"file": path, "error": e.to_string()}));
                    continue;
                }
            };
            let label = stem(path);
            s.edit("Contact Sheet", |doc, active| {
                let l = Layer::new(label.clone(), LayerContent::Raster(surf));
                *active = Some(l.id);
                doc.layers.push(l);
                Ok(())
            })?;
            placed += 1;
            if caption {
                let size_pt = font_pt;
                let baseline = y0 + box_h + caption_px * 0.72;
                s.execute("type.create", json!({"x": x0 + cell_w / 2.0, "y": baseline, "text": label, "font": font, "size": size_pt, "align": "center", "color": [0.0, 0.0, 0.0], "name": label}))?;
            }
        }
        if flatten {
            s.execute("layer.flattenImage", json!({}))?;
            // The reference ends a flattened sheet with a normal layer, not a Background (#13702).
            s.edit("Make Layer", |doc, active| {
                let l = doc.background_mut().ok_or(EngineError::Other("the flattened sheet has no Background layer".into()))?;
                crate::extra_cmds::unlock_background(l);
                l.name = "Contact Sheet II".into();
                *active = Some(l.id);
                Ok(())
            })?;
        }
        docs.push(idx);
    }
    Ok(json!({"documents": docs, "pages": docs.len(), "images": placed, "errors": errors, "width": pw, "height": ph}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    let native = crate::file_cmds::native;
    vec![
        spec!(
            "file.automate.contactSheetII",
            "Contact Sheet II…",
            &["File", "Automate"],
            r##"{"input":folder|[paths],"units":"inches|cm|mm|pixels"="inches","width":8,"height":10,"resolution":ppi=300,"mode":"rgb|gray|cmyk|lab"="rgb","depth":8|16=8,"columns":5,"rows":6,"placeAcrossFirst":bool=true,"autoSpacing":bool=true,"horizontal":units?,"vertical":units?,"rotateForBestFit":bool=false,"caption":bool=true (file name as caption),"font":family?,"fontSize":pt=12,"flatten":bool=false,"includeSubfolders":bool=true (a folder input also takes its subfolders' images, depth first after its own),"groupByFolder":bool=false (with includeSubfolders: each folder starts a new sheet)} → {documents, pages, images}"##,
            native,
            contact_sheet
        ),
        spec!(
            "file.automate.createDroplet",
            "Create Droplet…",
            &["File", "Automate"],
            r##"{"path":str (.pcdroplet),"steps":[[id,params]…]?,"action":name|index|[steps]? (used only when steps is absent; named actions exclude disabled steps),"set":str? (with a named action: that set's action of the name),"name":str?,"destination":"none|folder|saveAndClose"? (#13711, stored with the droplet; an omitted destination stores no key and the droplet runs as folder, as before),"output":folder? (stored only with a folder destination),"format":"same|png|jpg|…"?,"quality":0..12?,"overrideOpen":bool?,"includeSubfolders":bool?,"suppressOpenOptions":bool?,"suppressWarnings":bool? (the Play options, saved in the droplet and used by each run),"overrideSaveAs":bool? (the form's Override Action "Save As" Commands tick, stored; a run does not act on it yet),"shim":bool=true on Unix (writes <name>.command calling `photocraft-cli droplet`; a none droplet gets no shim - its headless run is refused)} → {path, shim} (the actions its Play Action and Conditional steps call are saved in the droplet)"##,
            native,
            create_droplet
        ),
        spec!(
            "file.automate.runDroplet",
            "Run Droplet",
            &[],
            r##"{"droplet":path,"input":[files or folders],"output":folder? (default: droplet's, else <input folder>/droplet-output)} → {files, errors} (an input whose output name was already written in the run goes to errors; the droplet's stored "destination" (#13711): folder and a stored or absent output write copies there as before, saveAndClose overwrites each input in its own format and takes no output, none opens each input in the running app, keeps it changed and writes nothing - refused in a headless session, where no app stays open; a folder gives its own images, or with the droplet's includeSubfolders also its subfolders' depth first, written flat into output, a later one replacing an earlier one of the same name)"##,
            native,
            run_droplet
        ),
        spec!(
            "file.scripts.statistics",
            "Statistics…",
            &["File", "Scripts"],
            r##"{"mode":"mean|median|maximum|minimum|range|summation|variance|standardDeviation|skewness|kurtosis|entropy"="median","input":folder|[paths],"paths":folder|[paths]? (older name for input),"align":bool=false (Auto-Align first)} → new document with one stack-mode smart object"##,
            native,
            statistics
        ),
        spec!(
            "file.scripts.browse",
            "Browse…",
            &["File", "Scripts"],
            r##"{"path":script file | "script":text | "steps":[[id,params]…]} (JSON action, or one `command.id {json}` per line) → {steps, ok, results}"##,
            |_| Ok(()),
            browse
        ),
        spec!(
            "file.scripts.scriptEventsManager",
            "Script Events Manager…",
            &["File", "Scripts"],
            r##"{"enabled":bool?,"add":{"event":"startApplication|newDocument|openDocument|saveDocument|closeDocument|print|export|everything","script":path?|"steps":[…]?,"name":str?}?,"remove":index?,"removeAll":bool?} → {enabled, bindings, events}"##,
            |_| Ok(()),
            script_events
        ),
    ]
}

#[cfg(test)]
#[path = "droplet_destination_tests.rs"]
mod droplet_destination_tests;
#[cfg(test)]
#[path = "droplet_play_options_tests.rs"]
mod droplet_play_options_tests;
#[cfg(test)]
#[path = "droplet_skip_non_images_tests.rs"]
mod droplet_skip_non_images_tests;
#[cfg(test)]
#[path = "automate_cmds/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "automate_cmds/contact_sheet_subfolders_tests.rs"]
mod contact_sheet_subfolders_tests;

#[cfg(test)]
#[path = "automate_cmds/contact_sheet_flatten_tests.rs"]
mod contact_sheet_flatten_tests;
