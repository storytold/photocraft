//! Create Droplet on Windows names a `.cmd` launcher next to the `.pcdroplet` that File
//! Explorer accepts drops on and that runs `photocraft-cli droplet` on the dropped paths
//! (#13718). Neighbours pinned here: the Unix `.command` shim stays byte-identical,
//! `shim: false` writes nothing, and the launcher forwards spaces, `&`, `%`, `^` and
//! non-ASCII paths unchanged.

use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-droplet-win-shim-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// Creates `dir/Rotate` (or the given `path`) with one rotation step.
fn create(dir: &str, path: String, shim: Option<bool>) -> Value {
    let mut p = json!({"path": path, "steps": [["image.imageRotation.90cw", {}]], "format": "png", "output": format!("{dir}/out")});
    if let Some(shim) = shim {
        p["shim"] = json!(shim);
    }
    Session::new().execute("file.automate.createDroplet", p).unwrap()
}

#[cfg(windows)]
#[test]
fn windows_create_droplet_default_writes_a_runnable_cmd_launcher() {
    let dir = tmp("default");
    let r = create(&dir, format!("{dir}/Rotate"), None);
    let droplet = r["path"].as_str().unwrap().to_string();
    let shim = r["shim"].as_str().unwrap_or_else(|| panic!("the Windows answer names the launcher in \"shim\": {r}"));
    assert!(shim.ends_with("Rotate.cmd"), "{shim}");
    assert!(std::path::Path::new(shim).exists(), "the launcher is written next to the droplet");
    let text = std::fs::read_to_string(shim).unwrap();
    assert!(text.starts_with("@echo off"), "runnable batch content: {text}");
    assert!(text.contains("photocraft-cli"), "the launcher runs the CLI: {text}");
    assert!(text.contains("%~dp0%~n0.pcdroplet"), "the launcher points at the droplet beside it: {text}");
    assert!(text.contains("%*"), "dropped paths are passed to the CLI: {text}");
    // The action is stored inside the droplet (#2794, verified here for this issue): a fresh
    // session with no saved actions still runs the written droplet.
    let v: Value = serde_json::from_slice(&std::fs::read(&droplet).unwrap()).unwrap();
    assert_eq!(v["action"]["steps"], json!([["image.imageRotation.90cw", {}]]), "{v}");
    let in_dir = format!(r"{dir}\in");
    std::fs::create_dir_all(&in_dir).unwrap();
    {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 2, "background": "#808080"})).unwrap();
        crate::file_cmds::save_doc(&s.active().unwrap().doc, &format!(r"{in_dir}\a.png"), None).unwrap();
    }
    let run = Session::new().execute("file.automate.runDroplet", json!({"droplet": droplet, "input": [in_dir]})).unwrap();
    assert_eq!(run["errors"], json!([]), "{run}");
    assert_eq!(run["files"].as_array().unwrap().len(), 1, "{run}");
}

#[test]
fn explicit_shim_true_writes_the_platform_launcher() {
    let dir = tmp("explicit-true");
    let r = create(&dir, format!("{dir}/Rotate"), Some(true));
    let shim = r["shim"].as_str().unwrap();
    assert!(std::path::Path::new(shim).exists(), "{shim}");
    let text = std::fs::read_to_string(shim).unwrap();
    if cfg!(windows) {
        assert!(shim.ends_with("Rotate.cmd"), "{shim}");
        assert!(text.starts_with("@echo off"), "{text}");
    } else {
        assert!(shim.ends_with("Rotate.command"), "{shim}");
        assert!(text.starts_with("#!/bin/sh"), "{text}");
    }
}

#[cfg(unix)]
#[test]
fn the_unix_command_shim_is_unchanged() {
    // Guard test, passes before and after: pattern 4/14, the Windows branch must not touch it.
    let dir = tmp("unix-unchanged");
    let r = create(&dir, format!("{dir}/Rotate"), Some(true));
    let droplet = r["path"].as_str().unwrap().to_string();
    let shim = r["shim"].as_str().unwrap();
    let text = std::fs::read_to_string(shim).unwrap();
    assert!(text.starts_with("#!/bin/sh\n# PhotoCraft droplet: runs the action on the files given (or dropped).\n"), "{text}");
    assert!(text.contains("if [ -n \"${PHOTOCRAFT_CLI:-}\" ]; then"), "PHOTOCRAFT_CLI keeps priority: {text}");
    assert!(text.contains("elif [ -x /Applications/PhotoCraft.app/Contents/MacOS/photocraft-cli ]; then"), "the bundle lookup stays: {text}");
    let abs = std::fs::canonicalize(&droplet).unwrap().to_string_lossy().into_owned();
    assert!(text.contains(&format!("exec \"$cli\" droplet '{abs}' \"$@\"")), "the canonicalized droplet path stays single-quoted: {text}");
}

#[test]
fn explicit_shim_false_writes_no_launcher() {
    // Guard test, passes before and after: the shim:false neighbour is unchanged.
    let dir = tmp("explicit-false");
    let r = create(&dir, format!("{dir}/Rotate"), Some(false));
    assert!(r["shim"].is_null(), "{r}");
    assert!(!std::path::Path::new(&format!("{dir}/Rotate.cmd")).exists());
    assert!(!std::path::Path::new(&format!("{dir}/Rotate.command")).exists());
}

#[cfg(windows)]
#[test]
fn windows_launcher_for_a_json_droplet_points_at_the_json_file() {
    let dir = tmp("json-droplet");
    let r = create(&dir, format!("{dir}/Rotate.json"), None);
    let shim = r["shim"].as_str().unwrap_or_else(|| panic!("the Windows answer names the launcher: {}", r["shim"]));
    assert!(shim.ends_with("Rotate.cmd"), "{shim}");
    let text = std::fs::read_to_string(shim).unwrap();
    assert!(text.contains("%~dp0%~n0.json"), "the launcher names the droplet's real extension: {text}");
}

#[cfg(windows)]
#[test]
fn windows_launcher_body_is_ascii_even_for_a_non_ascii_droplet_path() {
    // The launcher derives the droplet from its own location instead of embedding the path,
    // so cmd's console codepage never sees a non-ASCII byte in the file itself.
    let dir = tmp("ascii-body");
    let r = create(&dir, format!(r"{dir}\Träpfö & café 100%"), None);
    let shim = r["shim"].as_str().unwrap().to_string();
    let text = std::fs::read_to_string(&shim).unwrap();
    assert!(text.bytes().all(|b| b < 0x80), "the launcher body stays ASCII: {text}");
    assert!(text.contains("%~dp0%~n0.pcdroplet"), "{text}");
}

#[cfg(windows)]
#[test]
fn windows_launcher_runs_the_cli_on_every_dropped_path() {
    use std::os::windows::process::CommandExt;
    let dir = tmp("dropped-paths");
    let nasty = format!(r"{dir}\Café & 100% ^ (2)");
    std::fs::create_dir_all(&nasty).unwrap();
    let r = create(&nasty, format!(r"{nasty}\Droplet 1 & ^ 2%"), None);
    let launcher = format!(r"{nasty}\Droplet 1 & ^ 2%.cmd");
    assert!(std::path::Path::new(&launcher).exists(), "the launcher sits next to the droplet: {r}");

    // A tiny photocraft-cli stand-in, compiled with the toolchain running this test: it writes
    // the arguments it receives to a file. It must be a real exe, because calling a batch file
    // through cmd re-parses its command line and would not show what the CLI gets.
    let stub_rs = format!(r"{nasty}\cli stub.rs");
    let stub_exe = format!(r"{nasty}\cli stub.exe");
    std::fs::write(&stub_rs, "fn main() {\n    let a: Vec<String> = std::env::args().collect();\n    std::fs::write(std::env::var(\"STUB_OUT\").unwrap(), a[1..].join(\"\\n\")).unwrap();\n}\n").unwrap();
    let rustc = std::path::Path::new(env!("CARGO")).with_file_name("rustc");
    let build = std::process::Command::new(rustc)
        .arg("--edition")
        .arg("2021")
        .arg("--crate-name")
        .arg("cli_stub")
        .arg("-O")
        .arg(&stub_rs)
        .arg("-o")
        .arg(&stub_exe)
        .output()
        .unwrap();
    assert!(build.status.success(), "stub build: {}", String::from_utf8_lossy(&build.stderr));

    let inputs = [format!(r"{nasty}\img 100% & café (1)^.png"), format!(r"{dir}\bang ! second.jpg"), format!(r"{dir}\some folder & (x)")];
    std::fs::write(&inputs[0], "x").unwrap();
    std::fs::write(&inputs[1], "x").unwrap();
    std::fs::create_dir_all(&inputs[2]).unwrap();
    // Run it the way Explorer runs a `.cmd` a drop landed on: one command line
    // `cmd /d /c ""launcher" "path" …"`. (Spawning the batch directly with
    // `Command::new(launcher).arg(…)` goes through the CRT batch trampoline's own escaping,
    // documented as not safe for cmd metacharacters; Explorer builds the line itself.)
    let quoted: Vec<String> = inputs.iter().map(|p| format!("\"{p}\"")).collect();
    let output = std::process::Command::new("cmd.exe")
        .raw_arg(format!("/d /c \"\"{launcher}\" {}\"", quoted.join(" ")))
        .env("PHOTOCRAFT_CLI", &stub_exe)
        .env("STUB_OUT", format!(r"{nasty}\captured.txt"))
        .output()
        .unwrap();
    assert!(output.status.success(), "launcher exited {:?}: {}", output.status, String::from_utf8_lossy(&output.stderr));
    let captured = std::fs::read_to_string(format!(r"{nasty}\captured.txt")).unwrap();
    let lines: Vec<&str> = captured.split('\n').collect();
    assert_eq!(lines.len(), 5, "{captured}");
    assert_eq!(lines[0], "droplet", "the launcher calls the CLI's droplet subcommand");
    assert_eq!(lines[1], format!(r"{nasty}\Droplet 1 & ^ 2%.pcdroplet"), "the droplet's own path reaches the run unchanged");
    assert_eq!(lines[2], inputs[0], "spaces, &, %, ^ and non-ASCII survive");
    assert_eq!(lines[3], inputs[1]);
    assert_eq!(lines[4], inputs[2], "a dropped folder reaches the run as a folder path");
}

#[cfg(windows)]
#[test]
fn windows_launcher_self_location_survives_a_double_extension_name() {
    // Only the extension the launcher matched is stripped, once: `Rotate.json.pcdroplet`
    // gets `Rotate.json.cmd`, so `%~dp0%~n0.pcdroplet` names the droplet that exists.
    // Stripping both suffixes (trim_end_matches chains) lands on `Rotate.pcdroplet`, which
    // does not exist; the same for `Plain.json.json` -> `Plain.cmd` -> `Plain.json`.
    let dir = tmp("double-extension");
    let r = create(&dir, format!("{dir}/Rotate.json.pcdroplet"), None);
    let droplet = r["path"].as_str().unwrap().to_string();
    let shim = r["shim"].as_str().unwrap_or_else(|| panic!("the Windows answer names the launcher: {r}"));
    assert!(shim.ends_with("Rotate.json.cmd"), "{shim}");
    let text = std::fs::read_to_string(shim).unwrap();
    assert!(text.contains("%~dp0%~n0.pcdroplet"), "the droplet's real name keeps its inner .json: {text}");
    let self_located = format!("{}.pcdroplet", shim.strip_suffix(".cmd").unwrap());
    assert_eq!(std::fs::canonicalize(&self_located).unwrap(), std::fs::canonicalize(&droplet).unwrap(), "the launcher self-locates the droplet on disk");
    let r = create(&dir, format!("{dir}/Plain.json.json"), None);
    let droplet = r["path"].as_str().unwrap().to_string();
    let shim = r["shim"].as_str().unwrap();
    assert!(shim.ends_with("Plain.json.cmd"), "{shim}");
    let text = std::fs::read_to_string(shim).unwrap();
    assert!(text.contains("%~dp0%~n0.json"), "{text}");
    let self_located = format!("{}.json", shim.strip_suffix(".cmd").unwrap());
    assert_eq!(std::fs::canonicalize(&self_located).unwrap(), std::fs::canonicalize(&droplet).unwrap(), "the launcher self-locates the droplet on disk");
}
