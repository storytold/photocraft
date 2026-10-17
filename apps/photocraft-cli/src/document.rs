use std::io::Write;
use std::path::Path;

use photocraft_automation::{Headless, files};
use serde_json::{Value, json};

use crate::{
    R,
    args::Args,
    output::{export_opts, missing_font_warnings, print_json, warn_all},
};

pub(crate) fn convert(a: &Args, _out: &mut dyn Write, err: &mut dyn Write) -> R {
    let [input, output] = a.positional.as_slice() else {
        return Err("convert needs <in> <out>".into());
    };
    let opts = export_opts(a)?;
    let o = files::open(Path::new(input)).map_err(|e| e.to_string())?;
    warn_all(err, &o.warnings);
    warn_all(err, &photocraft_engine::type_extra_cmds::missing_font_warnings(&o.document));
    let ws = files::save(&o.document, Path::new(output), a.get("--format"), &opts, None).map_err(|e| e.to_string())?;
    warn_all(err, &ws);
    Ok(())
}

pub(crate) fn info(a: &Args, out: &mut dyn Write) -> R {
    let [file] = a.positional.as_slice() else {
        return Err("info needs <file>".into());
    };
    let mut h = Headless::trusted_local();
    let opened = h.open(Path::new(file)).map_err(|e| e.to_string())?;
    let mut doc = h.inspect(None).map_err(|e| e.to_string())?;
    let mut warnings = opened["warnings"].as_array().cloned().unwrap_or_default();
    if let Some(st) = h.session.active() {
        warnings.extend(missing_font_warnings(photocraft_engine::type_extra_cmds::missing_fonts(&st.doc)).into_iter().map(Value::String));
    }
    doc["warnings"] = Value::Array(warnings);
    doc["file"] = json!(file);
    // Drop per-session noise.
    if let Value::Object(m) = &mut doc {
        for k in ["history", "canUndo", "canRedo", "revision"] {
            m.remove(k);
        }
    }
    print_json(out, &doc, a.has("--compact"))
}

/// `(id, params)` pairs from repeated `--cmd`/`--params` flags.
fn command_list(a: &Args) -> Result<Vec<(String, Value)>, String> {
    let mut cmds: Vec<(String, Value)> = Vec::new();
    for (k, v) in &a.flags {
        match (k.as_str(), v) {
            ("--cmd", Some(id)) => cmds.push((id.clone(), json!({}))),
            ("--params", Some(p)) => {
                let last = cmds.last_mut().ok_or("--params must follow a --cmd")?;
                last.1 = serde_json::from_str(p).map_err(|e| format!("bad --params JSON for `{}`: {e}", last.0))?;
            }
            _ => {}
        }
    }
    Ok(cmds)
}

pub(crate) fn run_cmds(a: &Args, out: &mut dyn Write, err: &mut dyn Write) -> R {
    let opts = export_opts(a)?;
    let mut h = Headless::trusted_local();
    // Opening, running commands and exporting can all notice the same missing font:
    // one `run` invocation reports each warning line at most once.
    let mut reported_fonts = std::collections::HashSet::<String>::new();
    match (a.positional.as_slice(), a.get("--new")) {
        ([file], None) => {
            let o = h.open(Path::new(file)).map_err(|e| e.to_string())?;
            let ws: Vec<String> = serde_json::from_value(o["warnings"].clone()).unwrap_or_default();
            reported_fonts.extend(ws.iter().cloned());
            warn_all(err, &ws);
            reported_fonts.extend(ws);
        }
        ([], Some(new)) => {
            let p: Value = serde_json::from_str(new).map_err(|e| format!("bad --new JSON: {e}"))?;
            h.command_run("file.new", p).map_err(|e| format!("--new: {e}"))?;
        }
        _ => return Err("run needs exactly one of <file> or --new <json>".into()),
    }
    let cmds = command_list(a)?;
    if cmds.is_empty() && a.get("--out").is_none() {
        return Err("run needs at least one --cmd (or --out)".into());
    }
    for (id, params) in cmds {
        let r = h.command_run(&id, params).map_err(|e| format!("`{id}`: {e}"))?;
        if let Some(e) = photocraft_engine::actions_cmds::nested_failure(&id, &r) {
            return Err(format!("`{id}`: {e}"));
        }
        let new_warnings: Vec<String> = h
            .session
            .active()
            .map(|st| missing_font_warnings(photocraft_engine::type_extra_cmds::missing_fonts(&st.doc)))
            .unwrap_or_default()
            .into_iter()
            .filter(|w| reported_fonts.insert(w.clone()))
            .collect();
        warn_all(err, &new_warnings);
        let mut result = json!({"command": id, "result": r});
        if !new_warnings.is_empty() {
            result["warnings"] = json!(new_warnings);
        }
        print_json(out, &result, true)?;
    }
    // A `run` with only `--out` still reports missing fonts before exporting.
    let remaining: Vec<String> = h
        .session
        .active()
        .map(|st| missing_font_warnings(photocraft_engine::type_extra_cmds::missing_fonts(&st.doc)))
        .unwrap_or_default()
        .into_iter()
        .filter(|w| reported_fonts.insert(w.clone()))
        .collect();
    warn_all(err, &remaining);
    if let Some(o) = a.get("--out") {
        let r = h.save(None, Some(Path::new(o)), a.get("--format"), &opts).map_err(|e| e.to_string())?;
        let ws: Vec<String> = serde_json::from_value(r["warnings"].clone()).unwrap_or_default();
        let ws: Vec<String> = ws.into_iter().filter(|w| reported_fonts.insert(w.clone())).collect();
        warn_all(err, &ws);
    }
    Ok(())
}

#[cfg(test)]
mod missing_font_warning_tests {
    use super::*;
    use crate::{batch::is_input, run};

    fn invoke(args: &[&str]) -> (i32, String, String) {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let code = run(&args, &mut output, &mut errors);
        (code, String::from_utf8(output).unwrap(), String::from_utf8(errors).unwrap())
    }

    #[test]
    fn missing_typeface_is_reported_by_run_info_and_convert() {
        let family = "DefinitelyMissingPhotoCraftTypeface1251";
        let folder = std::env::temp_dir().join(format!("photocraft-cli-font-warnings-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let native = folder.join("missing.pcraft");
        let png = folder.join("missing.png");
        let native_str = native.to_string_lossy().to_string();
        let png_str = png.to_string_lossy().to_string();
        let params = json!({"x": 6, "y": 38, "text": "Missing font warning", "font": family, "size": 20}).to_string();

        let (code, stdout, stderr) = invoke(&[
            "run",
            "--new",
            r#"{"width":256,"height":96,"background":"white"}"#,
            "--cmd",
            "type.create",
            "--params",
            &params,
            "--cmd",
            "type.resolveMissingFonts",
            "--out",
            &native_str,
        ]);
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(stderr.matches(family).count(), 1, "multi-command runs should not repeat the same missing font warning");
        let commands: Vec<Value> = stdout.lines().map(|s| serde_json::from_str(s).unwrap()).collect();
        assert_eq!(commands.len(), 2, "{stdout}");
        assert!(commands[0]["warnings"].as_array().unwrap().iter().any(|w| w.as_str().is_some_and(|s| s.contains(family))));
        assert!(commands[1].get("warnings").is_none(), "only newly detected missing fonts are reported");

        let (code, stdout, stderr) = invoke(&["info", &native_str, "--compact"]);
        assert_eq!(code, 0, "{stderr}");
        let info: Value = serde_json::from_str(&stdout).unwrap();
        assert!(info["warnings"].as_array().unwrap().iter().any(|w| w.as_str().is_some_and(|s| s.contains(family))));
        assert!(stderr.is_empty(), "info puts warnings in JSON, not stderr");

        let (code, _, stderr) = invoke(&["convert", &native_str, &png_str]);
        assert_eq!(code, 0, "{stderr}");
        assert!(stderr.contains(&format!("warning: font '{family}'")), "{stderr}");
        assert!(png.is_file());

        // Even with no commands, run --out checks the opened document before export.
        let (code, stdout, stderr) = invoke(&["run", &native_str, "--out", &png_str]);
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.is_empty());
        assert_eq!(stderr.matches(family).count(), 1);

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn convert_reads_and_writes_openraster() {
        let folder = std::env::temp_dir().join(format!("photocraft-cli-ora-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = |name: &str| folder.join(name).to_string_lossy().to_string();
        let (native, ora, png) = (path("layers.pcraft"), path("layers.ora"), path("back.png"));
        let (code, _, stderr) = invoke(&["run", "--new", r#"{"width":16,"height":12}"#, "--cmd", "layer.new.layer", "--out", &native]);
        assert_eq!(code, 0, "{stderr}");
        let (code, _, stderr) = invoke(&["convert", &native, &ora]);
        assert_eq!(code, 0, "{stderr}");
        let (code, stdout, stderr) = invoke(&["info", &ora, "--compact"]);
        assert_eq!(code, 0, "{stderr}");
        let info: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(info["layers"].as_array().map(Vec::len), Some(2), "{info}");
        let (code, _, stderr) = invoke(&["convert", &ora, &png]);
        assert_eq!(code, 0, "{stderr}");
        assert!(is_input(std::path::Path::new(&ora)));
        // Krita documents are batch inputs too (import-only).
        for name in ["sketch.kra", "old.KRZ"] {
            std::fs::write(folder.join(name), b"").unwrap();
            assert!(is_input(&folder.join(name)), "{name}");
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn no_type_layers_do_not_produce_missing_font_warnings() {
        let folder = std::env::temp_dir().join(format!("photocraft-cli-no-font-warning-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let native = folder.join("blank.pcraft");
        let png = folder.join("blank.png");
        let native_str = native.to_string_lossy().to_string();
        let png_str = png.to_string_lossy().to_string();
        let (code, _, stderr) = invoke(&["run", "--new", r#"{"width":16,"height":16}"#, "--out", &native_str]);
        assert_eq!(code, 0, "{stderr}");
        assert!(!stderr.contains("font '"), "{stderr}");
        let (code, stdout, stderr) = invoke(&["info", &native_str, "--compact"]);
        assert_eq!(code, 0, "{stderr}");
        let info: Value = serde_json::from_str(&stdout).unwrap();
        assert!(info["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap_or_default().contains("font '")));
        let (code, _, stderr) = invoke(&["convert", &native_str, &png_str]);
        assert_eq!(code, 0, "{stderr}");
        assert!(!stderr.contains("font '"), "{stderr}");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
