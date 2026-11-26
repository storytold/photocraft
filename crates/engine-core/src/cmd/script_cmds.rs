//! File › Scripts: running a JavaScript file (`.js`, `.jsx`).
//!
//! The engine reads and checks what a run needs ([`resolve`]: the script's name and text). It
//! runs nothing: a run needs a script host and an interface for its dialogs, which the app has
//! and the engine does not. The app handles `file.scripts.run` itself; reaching the engine's own
//! handler (the CLI, an engine without the app) is an error that says so, never a success that
//! ran nothing.

use serde_json::Value;

use crate::commands::CommandSpec;
use crate::file_cmds::{extension, file_name, read_file_limited};
use crate::{EngineError, Result, Session};

/// Largest script a run reads.
pub const MAX_SCRIPT_BYTES: usize = 16 << 20;

/// Why a script can't run here.
pub const NEEDS_THE_APP: &str = "running a JavaScript script needs the PhotoCraft app; it was not run";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Whether `name` is a script the app runs: `.js` or `.jsx`. Compiled `.jsxbin` and UXP
/// scripts (`.psjs`, `.ccjs`) are refused with a reason.
///
/// # Errors
/// Why `name` is not a script the app runs.
pub fn runnable(cmd: &str, name: &str) -> Result<()> {
    match extension(name).as_deref() {
        Some("js" | "jsx") => Ok(()),
        Some("jsxbin") => Err(bad(cmd, "compiled ExtendScript (.jsxbin) can't be run; load the .jsx it was made from")),
        Some(ext @ ("psjs" | "ccjs")) => Err(bad(cmd, format!("UXP scripts (.{ext}) are not supported; PhotoCraft runs .js and .jsx scripts"))),
        _ => Err(bad(cmd, format!("{name} is not a script (.js or .jsx)"))),
    }
}

/// The text of the script file at `path`: at most [`MAX_SCRIPT_BYTES`], UTF-8, without a
/// byte-order mark.
///
/// # Errors
/// The file can't be read, is too large, or is not UTF-8 text.
pub fn read_script(cmd: &str, path: &str) -> Result<String> {
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

/// What a run of the script `p` names needs: its name and text. `p` is `{"path"}` (a `.js` or
/// `.jsx` file) or `{"name", "source"}` (a script's text, as the web gets it).
///
/// # Errors
/// The parameters name no script, or the script can't be read.
pub fn resolve(p: &Value) -> Result<(String, String)> {
    let cmd = "file.scripts.run";
    match (p.get("path").and_then(Value::as_str), p.get("source").and_then(Value::as_str)) {
        (Some(path), None) => {
            runnable(cmd, path)?;
            Ok((file_name(path), read_script(cmd, path)?))
        }
        (None, Some(source)) => {
            let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "a script's text needs its \"name\""))?;
            runnable(cmd, name)?;
            if source.len() > MAX_SCRIPT_BYTES {
                return Err(bad(cmd, format!("a script can be at most {} MiB", MAX_SCRIPT_BYTES >> 20)));
            }
            Ok((file_name(name), source.trim_start_matches('\u{feff}').to_owned()))
        }
        _ => Err(bad(cmd, "give \"path\" (a .js or .jsx file), or \"name\" and \"source\"")),
    }
}

/// `file.scripts.run` where the app did not take it: the script is checked, and the run refused.
fn run(_: &mut Session, p: &Value) -> Result<Value> {
    resolve(p)?;
    Err(EngineError::Other(NEEDS_THE_APP.into()))
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "file.scripts.run",
        label: "Run Script",
        menu: &[],
        shortcut: None,
        params: r#"{"path":str? (a .js or .jsx file),"name":str? (with source),"source":str?} (path, or name with source) → {script, running} (the app starts the run; without the app this is an error)"#,
        enabled: always,
        journal: false,
        run,
    }]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_script_is_read_without_its_byte_order_mark() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.jsx");
        std::fs::write(&path, "\u{feff}alert(1)").unwrap();
        let (name, source) = resolve(&json!({"path": path.to_str().unwrap()})).unwrap();
        assert_eq!((name.as_str(), source.as_str()), ("hello.jsx", "alert(1)"));
        assert_eq!(resolve(&json!({"name": "x.js", "source": "\u{feff}1"})).unwrap(), ("x.js".to_owned(), "1".to_owned()));
    }

    #[test]
    fn what_is_not_a_runnable_script_is_refused_with_its_reason() {
        for (p, why) in [
            (json!({}), "give"),
            (json!({"path": "/x/actions.json"}), "not a script"),
            (json!({"path": "/x/compiled.jsxbin"}), "jsxbin"),
            (json!({"path": "/x/uxp.psjs"}), "UXP"),
            (json!({"source": "1"}), "needs its \"name\""),
            (json!({"name": "x.js", "source": "x".repeat(MAX_SCRIPT_BYTES + 1)}), "at most"),
        ] {
            let e = resolve(&p).unwrap_err().to_string();
            assert!(e.contains(why), "{p}: {e}");
        }
    }

    /// Without the app nothing runs the script: the command says so instead of reporting a run.
    #[test]
    fn the_engine_alone_refuses_to_run_a_script() {
        let mut s = crate::session();
        let e = s.execute("file.scripts.run", json!({"name": "x.js", "source": "alert(1)"})).unwrap_err().to_string();
        assert!(e.contains(NEEDS_THE_APP), "{e}");
    }
}
