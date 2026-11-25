//! File › Scripts › Scripts Manager: the JavaScript files (`.js`, `.jsx`) the user loaded, with
//! their tags, kept in the preferences.
//!
//! The engine keeps the list and resolves what a run needs (`file.scripts.run`: the script's
//! name and text, and for a listed script the `app.name` its first tag sets). Running it is the
//! app's: a run needs the script host and an interface for its dialogs, which the engine has
//! neither of. Tags belong to the list, never to a run's parameters, so a recorded action (and an
//! `.atn` file written from it) holds the script but not its tags.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::file_cmds::{extension, file_name, read_file_limited};
use crate::{EngineError, Result, Session};

/// Scripts the list may hold.
pub const MAX_SCRIPTS: usize = 256;
/// Tags one script may have.
pub const MAX_TAGS: usize = 16;
/// Characters one tag may have.
pub const MAX_TAG_CHARS: usize = 64;
/// Largest script text kept in the preferences, for a script loaded without a path (the web).
/// The preferences are one browser storage item there (a few MB in all), so these stay small.
pub const MAX_STORED_SOURCE: usize = 256 << 10;
/// Largest total of the script texts the list keeps.
pub const MAX_STORED_TOTAL: usize = 512 << 10;
/// Largest script file a run reads.
pub const MAX_SCRIPT_BYTES: usize = 16 << 20;

/// One loaded script.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoadedScript {
    /// Shown in the list: the file name.
    pub name: String,
    /// The file, read again for every run (desktop).
    pub path: Option<String>,
    /// The text, for a script loaded without a path (the web).
    pub source: Option<String>,
    /// The first sets `app.name` for runs from the list.
    pub tags: Vec<String>,
}

/// The Scripts Manager's list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoadedScripts {
    pub entries: Vec<LoadedScript>,
    /// What scripts keep with `app.putCustomOptions` (see `script_options_cmds`).
    pub custom_options: crate::script_options_cmds::CustomOptions,
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Whether `name` is a script the app runs: `.js` or `.jsx`. Compiled `.jsxbin` and UXP
/// scripts (`.psjs`) are refused with a reason.
pub fn runnable(cmd: &str, name: &str) -> Result<()> {
    match extension(name).as_deref() {
        Some("js" | "jsx") => Ok(()),
        Some("jsxbin") => Err(bad(cmd, "compiled ExtendScript (.jsxbin) can't be run; load the .jsx it was made from")),
        Some("psjs" | "ccjs") => Err(bad(cmd, "UXP scripts (.psjs) are not supported; PhotoCraft runs .js and .jsx scripts")),
        _ => Err(bad(cmd, format!("{name} is not a script (.js or .jsx)"))),
    }
}

/// Tags from a list or a comma-separated text: trimmed, without empty or repeated ones.
pub fn tags_from(cmd: &str, v: &Value) -> Result<Vec<String>> {
    let raw: Vec<String> = match v {
        Value::String(s) => s.split(',').map(str::to_owned).collect(),
        Value::Array(a) => a.iter().map(|t| t.as_str().map(str::to_owned).ok_or_else(|| bad(cmd, "tags must be text"))).collect::<Result<_>>()?,
        Value::Null => Vec::new(),
        _ => return Err(bad(cmd, "tags are a list or comma-separated text")),
    };
    let mut tags: Vec<String> = Vec::new();
    for t in raw {
        let t = t.trim();
        if t.is_empty() || tags.iter().any(|have| have == t) {
            continue;
        }
        if t.chars().count() > MAX_TAG_CHARS {
            return Err(bad(cmd, format!("a tag can have at most {MAX_TAG_CHARS} characters")));
        }
        if tags.len() == MAX_TAGS {
            return Err(bad(cmd, format!("a script can have at most {MAX_TAGS} tags")));
        }
        tags.push(t.to_owned());
    }
    Ok(tags)
}

fn index(cmd: &str, s: &Session, p: &Value) -> Result<usize> {
    let i = crate::commands::int(p, "index").ok_or_else(|| bad(cmd, "give \"index\" (a script in the list)"))?;
    usize::try_from(i).ok().filter(|i| *i < s.prefs().scripts.entries.len()).ok_or_else(|| bad(cmd, format!("no script {i} in the list")))
}

/// The list as the commands report it (without stored texts).
fn listing(s: &Session) -> Value {
    let entries: Vec<Value> = s
        .prefs()
        .scripts
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| json!({"index": i, "name": e.name, "path": e.path, "tags": e.tags, "stored": e.source.is_some()}))
        .collect();
    json!({ "scripts": entries })
}

/// `file.scripts.manager`: the list (the app opens the Scripts Manager).
fn manager(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(listing(s))
}

/// `file.scripts.load`: add a script file (`path`), or a script's text (`name` and `source`).
fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.scripts.load";
    let tags = tags_from(cmd, p.get("tags").unwrap_or(&Value::Null))?;
    let entry = match (p.get("path").and_then(Value::as_str), p.get("source").and_then(Value::as_str)) {
        (Some(path), _) => {
            if path.is_empty() {
                return Err(bad(cmd, "the path is empty"));
            }
            runnable(cmd, path)?;
            // A script that can't be read is refused now rather than at its first run.
            read_script(cmd, path)?;
            LoadedScript { name: file_name(path), path: Some(path.to_owned()), source: None, tags }
        }
        (None, Some(source)) => {
            let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "a script's text needs its \"name\""))?;
            runnable(cmd, name)?;
            if source.len() > MAX_STORED_SOURCE {
                return Err(bad(cmd, format!("a script kept in the list can be at most {} KiB; load it from a file", MAX_STORED_SOURCE >> 10)));
            }
            let source = source.trim_start_matches('\u{feff}');
            let kept: usize = s.prefs().scripts.entries.iter().filter_map(|e| e.source.as_ref()).map(String::len).sum();
            if kept + source.len() > MAX_STORED_TOTAL {
                return Err(bad(
                    cmd,
                    format!("the list can keep at most {} KiB of script text; remove a script or run this one with Browse…", MAX_STORED_TOTAL >> 10),
                ));
            }
            LoadedScript { name: file_name(name), path: None, source: Some(source.to_owned()), tags }
        }
        (None, None) => return Err(bad(cmd, "give \"path\" (a .js or .jsx file), or \"name\" and \"source\"")),
    };
    // Loading a file again keeps its place and its tags, unless new ones are given.
    let existing = entry.path.as_ref().and_then(|path| s.prefs().scripts.entries.iter().position(|e| e.path.as_ref() == Some(path)));
    let at = match existing {
        Some(i) => {
            if p.get("tags").is_some() {
                s.edit_prefs(|pr| {
                    if let Some(e) = pr.scripts.entries.get_mut(i) {
                        e.tags = entry.tags.clone();
                    }
                });
            }
            i
        }
        None => {
            if s.prefs().scripts.entries.len() >= MAX_SCRIPTS {
                return Err(bad(cmd, format!("the list can hold at most {MAX_SCRIPTS} scripts")));
            }
            s.edit_prefs(|pr| pr.scripts.entries.push(entry));
            s.prefs().scripts.entries.len() - 1
        }
    };
    let mut out = listing(s);
    out["index"] = json!(at);
    Ok(out)
}

/// `file.scripts.remove`: take a script off the list (the file stays).
fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let i = index("file.scripts.remove", s, p)?;
    s.edit_prefs(|pr| {
        if i < pr.scripts.entries.len() {
            pr.scripts.entries.remove(i);
        }
    });
    Ok(listing(s))
}

/// `file.scripts.setTags`: replace a listed script's tags.
fn set_tags(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.scripts.setTags";
    let i = index(cmd, s, p)?;
    let tags = tags_from(cmd, p.get("tags").unwrap_or(&Value::Null))?;
    s.edit_prefs(|pr| {
        if let Some(e) = pr.scripts.entries.get_mut(i) {
            e.tags = tags;
        }
    });
    Ok(listing(s))
}

fn read_script(cmd: &str, path: &str) -> Result<String> {
    // A larger file is refused before it is read.
    let bytes = read_file_limited(path, MAX_SCRIPT_BYTES as u64).map_err(|e| match e.kind() {
        std::io::ErrorKind::FileTooLarge | std::io::ErrorKind::OutOfMemory => {
            bad(cmd, format!("{} is larger than {} MiB", file_name(path), MAX_SCRIPT_BYTES >> 20))
        }
        _ => EngineError::Other(format!("{path}: {e}")),
    })?;
    // ExtendScript files are often saved with a byte-order mark.
    let text = String::from_utf8(bytes).map_err(|_| bad(cmd, format!("{} is not UTF-8 text", file_name(path))))?;
    Ok(text.strip_prefix('\u{feff}').map(str::to_owned).unwrap_or(text))
}

/// `file.scripts.run`: what a run of a script needs. The app runs it; on its own (the CLI, an
/// agent without the app) this only resolves and checks the script.
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.scripts.run";
    let (name, source) = match (p.get("index"), p.get("path").and_then(Value::as_str), p.get("source").and_then(Value::as_str)) {
        (Some(_), _, _) => {
            let i = index(cmd, s, p)?;
            let e = s.prefs().scripts.entries.get(i).cloned().ok_or_else(|| bad(cmd, format!("no script {i} in the list")))?;
            let source = match (&e.path, &e.source) {
                (Some(path), _) => read_script(cmd, path)?,
                (None, Some(text)) => text.clone(),
                (None, None) => return Err(bad(cmd, "the listed script has neither a file nor a text")),
            };
            // Tags are the user's: the app passes the first one for a run from the Scripts
            // Manager, never for a recorded step.
            (e.name, source)
        }
        (None, Some(path), _) => {
            runnable(cmd, path)?;
            (file_name(path), read_script(cmd, path)?)
        }
        (None, None, Some(source)) => {
            let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "a script's text needs its \"name\""))?;
            runnable(cmd, name)?;
            if source.len() > MAX_SCRIPT_BYTES {
                return Err(bad(cmd, format!("a script can be at most {} MiB", MAX_SCRIPT_BYTES >> 20)));
            }
            (file_name(name), source.trim_start_matches('\u{feff}').to_owned())
        }
        _ => return Err(bad(cmd, "give \"index\" (a listed script), \"path\" (a .js or .jsx file), or \"name\" and \"source\"")),
    };
    Ok(json!({ "name": name, "source": source }))
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $params:expr, $journal:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: always, journal: $journal, run: $run }
        };
    }
    vec![
        spec!(
            "file.scripts.manager",
            "Scripts Manager…",
            &["File", "Scripts"],
            r#"{} → {scripts: [{index, name, path, tags, stored}]} (the app opens the Scripts Manager)"#,
            false,
            manager
        ),
        spec!(
            "file.scripts.load",
            "Load Script",
            &[],
            r#"{"path":str? (a .js or .jsx file),"name":str? (with source, instead of path),"source":str? (at most 256 KiB; 512 KiB for all texts kept),"tags":[str]|str? ("a, b")} → {scripts, index} (a file already listed keeps its place)"#,
            false,
            load
        ),
        spec!("file.scripts.remove", "Remove Script", &[], r#"{"index": int} → {scripts}"#, false, remove),
        spec!(
            "file.scripts.setTags",
            "Set Script Tags",
            &[],
            r#"{"index": int, "tags": [str] | "a, b"} → {scripts} (the first tag is app.name for runs from the Scripts Manager)"#,
            false,
            set_tags
        ),
        spec!(
            "file.scripts.run",
            "Run Script",
            &[],
            r#"{"index":int? (a listed script),"path":str? (a .js or .jsx file),"name":str? (with source),"source":str?} (one of index, path, or name with source) → {name, source} (the app runs it, with app.name from the first tag when run from the Scripts Manager; headless this only resolves it)"#,
            true,
            run
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        crate::session()
    }

    #[test]
    fn tags_are_trimmed_deduplicated_and_bounded() {
        assert_eq!(tags_from("t", &json!(" Photoshop, PS ,,Photoshop")).unwrap(), ["Photoshop", "PS"]);
        assert_eq!(tags_from("t", &json!(["a", " b "])).unwrap(), ["a", "b"]);
        assert!(tags_from("t", &json!([1])).is_err());
        assert!(tags_from("t", &json!("x".repeat(MAX_TAG_CHARS + 1))).is_err());
        let many: Vec<String> = (0..=MAX_TAGS).map(|i| i.to_string()).collect();
        assert!(tags_from("t", &json!(many)).is_err());
        assert!(tags_from("t", &json!({"a": 1})).is_err());
    }

    #[test]
    fn a_stored_script_loads_runs_with_its_first_tag_and_is_removed() {
        let mut s = session();
        let r = s.execute("file.scripts.load", json!({"name": "hello.jsx", "source": "\u{feff}alert(app.name)", "tags": "Photoshop, Other"})).unwrap();
        assert_eq!(r["index"], 0);
        assert_eq!(r["scripts"][0], json!({"index": 0, "name": "hello.jsx", "path": null, "tags": ["Photoshop", "Other"], "stored": true}));
        let run = s.execute("file.scripts.run", json!({"index": 0})).unwrap();
        assert_eq!(run, json!({"name": "hello.jsx", "source": "alert(app.name)"}), "tags stay out of a resolved run");
        s.execute("file.scripts.setTags", json!({"index": 0, "tags": []})).unwrap();
        assert_eq!(s.prefs().scripts.entries[0].tags, Vec::<String>::new());
        let by_text = s.execute("file.scripts.run", json!({"name": "x.js", "source": "\u{feff}1"})).unwrap();
        assert_eq!(by_text, json!({"name": "x.js", "source": "1"}));
        assert_eq!(s.execute("file.scripts.remove", json!({"index": 0})).unwrap(), json!({"scripts": []}));
        assert!(s.prefs().scripts.entries.is_empty());
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = session();
        for (id, p) in [
            ("file.scripts.load", json!({})),
            ("file.scripts.load", json!({"path": ""})),
            ("file.scripts.load", json!({"path": "/x/actions.json"})),
            ("file.scripts.load", json!({"path": "/x/compiled.jsxbin"})),
            ("file.scripts.load", json!({"path": "/x/plugin.psjs"})),
            ("file.scripts.load", json!({"source": "1"})),
            ("file.scripts.load", json!({"name": "big.js", "source": "x".repeat(MAX_STORED_SOURCE + 1)})),
            ("file.scripts.load", json!({"name": "a.js", "source": "1", "tags": 5})),
            ("file.scripts.remove", json!({"index": 0})),
            ("file.scripts.remove", json!({"index": -1})),
            ("file.scripts.setTags", json!({"index": 3, "tags": "a"})),
            ("file.scripts.run", json!({})),
            ("file.scripts.run", json!({"index": "0"})),
            ("file.scripts.run", json!({"path": "/no/such/file.jsx"})),
            ("file.scripts.run", json!({"name": "notes.txt", "source": "1"})),
        ] {
            assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
        }
    }

    #[test]
    fn a_script_file_over_the_limit_is_refused_before_it_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.jsx");
        std::fs::File::create(&path).unwrap().set_len(MAX_SCRIPT_BYTES as u64 + 1).unwrap();
        let path = path.to_string_lossy().into_owned();
        for id in ["file.scripts.load", "file.scripts.run"] {
            let err = session().execute(id, json!({ "path": path })).unwrap_err().to_string();
            assert!(err.contains("larger than 16 MiB"), "{id}: {err}");
        }
    }

    #[test]
    fn the_list_is_bounded() {
        let mut s = session();
        for i in 0..MAX_SCRIPTS {
            s.execute("file.scripts.load", json!({"name": format!("{i}.js"), "source": "1"})).unwrap();
        }
        assert!(s.execute("file.scripts.load", json!({"name": "one-more.js", "source": "1"})).is_err());
    }

    #[test]
    fn the_texts_the_list_keeps_are_bounded_in_total() {
        let mut s = session();
        let text = "x".repeat(MAX_STORED_SOURCE);
        for i in 0..MAX_STORED_TOTAL / MAX_STORED_SOURCE {
            s.execute("file.scripts.load", json!({"name": format!("{i}.js"), "source": text})).unwrap();
        }
        let err = s.execute("file.scripts.load", json!({"name": "more.js", "source": "1"})).unwrap_err().to_string();
        assert!(err.contains("512 KiB"), "{err}");
        s.execute("file.scripts.remove", json!({"index": 0})).unwrap();
        s.execute("file.scripts.load", json!({"name": "more.js", "source": "1"})).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_script_file_is_read_for_every_run_and_loading_it_again_keeps_its_place() {
        let dir = std::env::temp_dir().join(format!("photocraft-scripts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tool.jsx");
        std::fs::write(&path, "1").unwrap();
        let path = path.to_string_lossy().into_owned();
        let mut s = session();
        s.execute("file.scripts.load", json!({"path": path, "tags": ["Tagged"]})).unwrap();
        std::fs::write(&path, "2").unwrap();
        assert_eq!(s.execute("file.scripts.run", json!({"index": 0})).unwrap()["source"], "2");
        let again = s.execute("file.scripts.load", json!({"path": path})).unwrap();
        assert_eq!((again["index"].as_u64(), again["scripts"][0]["tags"].clone()), (Some(0), json!(["Tagged"])));
        assert_eq!(s.execute("file.scripts.run", json!({"path": path})).unwrap()["appName"], Value::Null, "a run by path has no tag");
        std::fs::write(&path, [0xFF, 0xFE, 0x00]).unwrap();
        assert!(s.execute("file.scripts.run", json!({"index": 0})).is_err(), "not UTF-8");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
