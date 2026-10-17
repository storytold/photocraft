use std::io::Write;
use std::path::{Path, PathBuf};

use photocraft_automation::Headless;
use serde_json::json;

use crate::{
    R,
    actions::load_actions_file,
    args::{BatchArgs, DropletArgs},
    output::{export_opts, warn_all},
};

/// Whether `a` and `b` name the same existing folder, however they are spelt (relative, with a
/// trailing separator, through a symbolic link).
fn same_dir(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

pub(crate) fn is_input(p: &Path) -> bool {
    let known = |e: &str| {
        matches!(e, "pcraft" | "psd" | "psb" | "pdn" | "ora" | "kra" | "krz" | "af" | "afdesign" | "afphoto" | "afpub")
            || photocraft_codecs::from_extension(e).is_some_and(|f| photocraft_codecs::caps(f).read)
    };
    p.is_file() && p.extension().is_some_and(|e| known(&e.to_string_lossy().to_ascii_lowercase()))
}

pub(crate) fn batch(a: &BatchArgs, out: &mut dyn Write, err: &mut dyn Write) -> R {
    let actions_path = a.actions.as_deref().ok_or("batch needs --actions <file>")?;
    let in_dir = PathBuf::from(a.in_dir.as_deref().ok_or("batch needs --in <dir>")?);
    let out_dir = PathBuf::from(a.out.as_deref().ok_or("batch needs --out <dir>")?);
    let (actions, set, warnings) = load_actions_file(actions_path, a.action.as_deref())?;
    for w in &warnings {
        let _ = writeln!(err, "warning: {w}");
    }
    let opts = export_opts(&a.export)?;
    // Results are saved as `<out>/<stem>.<ext>`, so an `--out` that is the `--in` folder would
    // replace the originals (#492).
    if !a.in_place && same_dir(&in_dir, &out_dir) {
        return Err(format!(
            "--out {} is the --in folder, so the results would replace the originals: choose another --out folder, or pass --in-place to overwrite them",
            out_dir.display()
        ));
    }
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let mut inputs: Vec<PathBuf> =
        std::fs::read_dir(&in_dir).map_err(|e| format!("{}: {e}", in_dir.display()))?.flatten().map(|e| e.path()).filter(|p| is_input(p)).collect();
    inputs.sort();
    let (mut ok, mut failed) = (0, 0);
    let mut written = photocraft_engine::file_cmds::OutputClaims::default();
    // `--format .jpg` names outputs `<stem>.jpg`, as `--format jpg` does (#490).
    let format = a.export.format.as_deref().map(|f| f.strip_prefix('.').unwrap_or(f));
    for input in &inputs {
        let ext = format.map(str::to_owned).or_else(|| input.extension().map(|e| e.to_string_lossy().into_owned())).unwrap_or_else(|| "png".into());
        let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let target = out_dir.join(format!("{stem}.{ext}"));
        let target_text = target.to_string_lossy();
        let r = (|| -> Result<Vec<String>, String> {
            written.check(&target_text)?;
            let mut h = Headless::trusted_local();
            // Steps that play another action find it in the set; the session's playback stack
            // stops an action that calls itself.
            h.session.actions.list = set.clone();
            h.open(input).map_err(|e| e.to_string())?;
            for (id, p) in &actions {
                if photocraft_engine::actions_cmds::shell_view_command(id) || photocraft_engine::actions_cmds::shell_save_command(id) {
                    continue;
                }
                let r = h.command_run(id, p.clone()).map_err(|e| format!("`{id}`: {e}"))?;
                // A failed step inside a called action makes this file an error, not a saved result.
                if let Some(e) = photocraft_engine::actions_cmds::nested_failure(id, &r) {
                    return Err(format!("`{id}`: {e}"));
                }
            }
            let r = h.save(None, Some(&target), Some(&ext), &opts).map_err(|e| e.to_string())?;
            Ok(serde_json::from_value(r["warnings"].clone()).unwrap_or_default())
        })();
        match r {
            Ok(ws) => {
                ok += 1;
                written.record(&target_text, &input.to_string_lossy());
                let _ = writeln!(out, "ok    {} -> {}", input.display(), target.display());
                warn_all(err, &ws);
            }
            Err(e) => {
                failed += 1;
                let _ = writeln!(err, "FAIL  {}: {e}", input.display());
            }
        }
    }
    let _ = writeln!(out, "{ok} succeeded, {failed} failed");
    if failed > 0 { Err(format!("{failed} file(s) failed")) } else { Ok(()) }
}

pub(crate) fn droplet(a: &DropletArgs, out: &mut dyn Write, err: &mut dyn Write) -> R {
    let (file, inputs) = (&a.file, &a.inputs);
    let mut p = json!({"droplet": file, "input": inputs});
    if let Some(o) = &a.out {
        p["output"] = json!(o);
    }
    let mut s = photocraft_engine::Session::new();
    let r = s.execute("file.automate.runDroplet", p).map_err(|e| e.to_string())?;
    for f in r["files"].as_array().into_iter().flatten() {
        let _ = writeln!(out, "ok    {}", f.as_str().unwrap_or_default());
    }
    let errors = r["errors"].as_array().cloned().unwrap_or_default();
    for e in &errors {
        let _ = writeln!(err, "FAIL  {}: {}", e["file"].as_str().unwrap_or_default(), e["error"].as_str().unwrap_or_default());
    }
    if errors.is_empty() { Ok(()) } else { Err(format!("{} file(s) failed", errors.len())) }
}

#[cfg(test)]
mod batch_action_set_tests {
    use super::*;
    use crate::run;
    use serde_json::Value;

    fn invoke(args: &[&str]) -> (i32, String, String) {
        let (mut output, mut errors) = (Vec::new(), Vec::new());
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let code = run(&args, &mut output, &mut errors);
        (code, String::from_utf8(output).unwrap(), String::from_utf8(errors).unwrap())
    }

    /// A batched action that plays another action of its set finds it in every file's session; a
    /// failure inside the called action fails the file, and a self-call stops instead of looping.
    #[test]
    fn batch_plays_an_action_that_calls_another_action_of_its_set() {
        let dir = std::env::temp_dir().join(format!("photocraft-cli-action-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = |name: &str| dir.join(name).to_string_lossy().to_string();
        std::fs::create_dir_all(dir.join("in")).unwrap();
        for name in ["a", "b"] {
            let (code, _, stderr) = invoke(&["run", "--new", r#"{"width":8,"height":6}"#, "--out", &path(&format!("in/{name}.png"))]);
            assert_eq!(code, 0, "{stderr}");
        }
        let set = json!({"version": 1, "actions": [
            {"name": "Outer", "steps": [["actions.play", {"action": "Rotate"}]]},
            {"name": "Rotate", "steps": [["image.imageRotation.90cw", {}]]},
            {"name": "Calls bad", "steps": [["actions.play", {"action": "Bad"}]]},
            {"name": "Bad", "steps": [["layer.delete", {"layer": 999_999}]]},
            {"name": "Loop", "steps": [["actions.play", {"action": "Loop"}]]},
        ]});
        std::fs::write(dir.join("actions.json"), set.to_string()).unwrap();
        let batch = |action: &str| invoke(&["batch", "--actions", &path("actions.json"), "--action", action, "--in", &path("in"), "--out", &path(action)]);

        let (code, stdout, stderr) = batch("Outer");
        assert_eq!(code, 0, "{stdout}{stderr}");
        for name in ["a", "b"] {
            let (code, info, stderr) = invoke(&["info", &path(&format!("Outer/{name}.png")), "--compact"]);
            assert_eq!(code, 0, "{stderr}");
            let info: Value = serde_json::from_str(&info).unwrap();
            assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(6), Some(8)), "rotated by the called action");
        }
        for action in ["Calls bad", "Loop"] {
            let (code, stdout, stderr) = batch(action);
            assert_ne!(code, 0, "{stdout}");
            assert_eq!(stderr.matches("FAIL").count(), 2, "{stderr}");
            assert!(!dir.join(action).join("a.png").exists(), "a file whose called action failed is not saved");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
