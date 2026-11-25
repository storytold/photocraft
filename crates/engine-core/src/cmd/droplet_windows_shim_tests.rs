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
    crate::session().execute("file.automate.createDroplet", p).unwrap()
}

#[cfg(windows)]
/// Compile a photocraft-cli stand-in that writes what it receives to `%STUB_OUT%`. It must be
/// a real exe: cmd re-parses the command line of a called batch file, so a `.cmd` stand-in
/// would not show what the CLI actually gets.
fn compile_stub(dir: &str, name: &str, src: &str) -> String {
    let stub_rs = format!(r"{dir}\{name}.rs");
    let stub_exe = format!(r"{dir}\{name}.exe");
    std::fs::write(&stub_rs, src).unwrap();
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
    stub_exe
}

#[cfg(windows)]
/// Stand-in recording only the arguments, one per line.
fn cli_stub(dir: &str) -> String {
    compile_stub(
        dir,
        "cli stub",
        "fn main() {\n    let a: Vec<String> = std::env::args().collect();\n    std::fs::write(std::env::var(\"STUB_OUT\").unwrap(), a[1..].join(\"\\n\")).unwrap();\n}\n",
    )
}

#[cfg(windows)]
/// Stand-in whose first line is the exe that ran, so one capture file also shows WHICH
/// photocraft-cli.exe the launcher picked.
fn cli_stub_who(dir: &str) -> String {
    compile_stub(
        dir,
        "cli who",
        "fn main() {\n    let a: Vec<String> = std::env::args().collect();\n    let who = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();\n    let mut out = vec![who];\n    out.extend(a[1..].iter().cloned());\n    std::fs::write(std::env::var(\"STUB_OUT\").unwrap(), out.join(\"\\n\")).unwrap();\n}\n",
    )
}

#[cfg(windows)]
/// Copy the stand-in to `<root>\<subdir>\photocraft-cli.exe`, as an installer would: the
/// subdir is `Programs\ArtCraft\PhotoCraft` under LOCALAPPDATA and `ArtCraft\PhotoCraft`
/// under Program Files.
fn install_stub(root: &str, subdir: &str, exe: &str) -> String {
    let dir = format!(r"{root}\{subdir}");
    std::fs::create_dir_all(&dir).unwrap();
    let exe_at = format!(r"{dir}\photocraft-cli.exe");
    std::fs::copy(exe, &exe_at).unwrap();
    exe_at
}

#[cfg(windows)]
/// One Explorer-shaped launcher run, `cmd /d /c ""launcher" "input""`, with faked CLI locations:
/// `local` / `machine` / `x86` become `%LOCALAPPDATA%` / `%ProgramFiles%` /
/// `%ProgramFiles(x86)%` (empty = defined, nothing in it), `path_dir` is the only PATH entry,
/// `cli_env` sets `PHOTOCRAFT_CLI`. Returns what the CLI recorded and everything cmd itself
/// printed.
fn run_lookup_probe(
    launcher: &str,
    input: &str,
    local: Option<&str>,
    machine: Option<&str>,
    x86: Option<&str>,
    path_dir: Option<&str>,
    cli_env: Option<&str>,
) -> (String, String) {
    use std::os::windows::process::CommandExt;
    let captured = format!(r"{launcher}.captured.txt");
    let _ = std::fs::remove_file(&captured);
    // cmd.exe sets %ProgramFiles% and %ProgramFiles(x86)% from the registry when it starts, so a
    // per-machine install folder cannot be faked in the child's environment: a one-line wrapper
    // sets them inside the same cmd process and calls the launcher with the same quoted argument.
    // The wrapper's own source stays ASCII: the launcher, input and fake install paths arrive
    // through the child's environment (handed to cmd.exe as UTF-16 by the spawner) and are only
    // expanded inside the batch. Embedding them in the .cmd text instead would decode them with
    // the console codepage the file is read in, which corrupts a non-ASCII TEMP path (a user
    // like Jose with an accent) whenever that codepage is not UTF-8.
    let mut invoked = launcher.to_string();
    let has_machine = machine.is_some_and(|m| !m.is_empty());
    let has_x86 = x86.is_some_and(|x| !x.is_empty());
    if has_machine || has_x86 {
        let wrapper = format!(r"{launcher}.wrapper.cmd");
        let mut sets = String::new();
        if has_machine {
            sets.push_str("set \"ProgramFiles=%PCW_MACHINE%\"\r\n");
        }
        if has_x86 {
            sets.push_str("set \"ProgramFiles(x86)=%PCW_X86%\"\r\n");
        } else {
            // No fake asked for: clear the registry's own folder, so a PhotoCraft installed on the
            // test machine can never be what a leg below the one under test picks.
            sets.push_str("set \"ProgramFiles(x86)=\"\r\n");
        }
        std::fs::write(&wrapper, format!("@echo off\r\n{sets}call \"%PCW_LAUNCHER%\" \"%PCW_INPUT%\"\r\n")).unwrap();
        invoked = wrapper;
    }
    let arg_line = if invoked == launcher { format!(" \"{input}\"") } else { String::new() };
    let mut cmd = std::process::Command::new("cmd.exe");
    cmd.raw_arg(format!("/d /c \"\"{invoked}\"{arg_line}\""))
        .env("PCW_LAUNCHER", launcher)
        .env("PCW_INPUT", input)
        .env("PCW_MACHINE", machine.unwrap_or(""))
        .env("PCW_X86", x86.unwrap_or(""))
        .env("LOCALAPPDATA", local.unwrap_or(""))
        .env("ProgramFiles", machine.unwrap_or(""))
        .env("ProgramFiles(x86)", x86.unwrap_or(""))
        .env("PATH", path_dir.unwrap_or(r"C:\Windows\System32"))
        .env("STUB_OUT", &captured)
        .env_remove("PC_CLI");
    match cli_env {
        Some(cli) => {
            cmd.env("PHOTOCRAFT_CLI", cli);
        }
        None => {
            cmd.env_remove("PHOTOCRAFT_CLI");
        }
    }
    let out = cmd.output().unwrap();
    let text = std::fs::read_to_string(&captured).unwrap_or_default();
    (text, format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
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
    assert!(text.contains("setlocal DisableDelayedExpansion"), "an exclamation mark in a path stays literal even when cmd has delayed expansion on: {text}");
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
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 4, "height": 2, "background": "#808080"})).unwrap();
        crate::file_cmds::save_doc(&s.active().unwrap().doc, &format!(r"{in_dir}\a.png"), None).unwrap();
    }
    let run = crate::session().execute("file.automate.runDroplet", json!({"droplet": droplet, "input": [in_dir]})).unwrap();
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
    // write_shim escapes an apostrophe in the path by the shell quote-splice rule; the
    // expectation applies the same escape, so a TMPDIR like /tmp/o'connor still matches the
    // correctly escaped launcher content instead of failing on it.
    let quoted_abs = abs.replace('\'', "'\\''");
    assert!(text.contains(&format!("exec \"$cli\" droplet '{quoted_abs}' \"$@\"")), "the canonicalized droplet path stays single-quoted: {text}");
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
    let stub_exe = cli_stub(&nasty);

    let inputs = [format!(r"{nasty}\img 100% & café (1)^.png"), format!(r"{dir}\bang ! second.jpg"), format!(r"{dir}\some folder & (x)")];
    std::fs::write(&inputs[0], "x").unwrap();
    std::fs::write(&inputs[1], "x").unwrap();
    std::fs::create_dir_all(&inputs[2]).unwrap();
    // Run it the way Explorer runs a `.cmd` a drop landed on: one command line
    // `cmd /d /c ""launcher" "path" …"`. (Spawning the batch directly with
    // `Command::new(launcher).arg(…)` goes through the CRT batch trampoline's own escaping,
    // documented as not safe for cmd metacharacters; Explorer builds the line itself.)
    // Explorer quotes a dropped path only when it contains a space, which is what this line
    // pins; the unquoted form is pinned by `windows_launcher_splits_an_unquoted_dropped_path`.
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

#[cfg(windows)]
#[test]
fn windows_launcher_finds_the_cli_in_the_per_user_install_folder_before_path() {
    // The Windows installer puts photocraft-cli.exe in the install folder and does not add it
    // to PATH, so on a default install a drop must find it there (#13718, comment 7025990366).
    // Order: PHOTOCRAFT_CLI, per-user install, per-machine install, PATH.
    let dir = tmp("install-lookup");
    let stub = cli_stub_who(&dir);
    create(&dir, format!("{dir}/Rotate"), None);
    let launcher = format!(r"{dir}\Rotate.cmd");
    std::fs::write(format!(r"{dir}\a.png"), "x").unwrap();
    install_stub(&format!(r"{dir}\fake-local"), r"Programs\ArtCraft\PhotoCraft", &stub);
    install_stub(&format!(r"{dir}\fake-progfiles"), r"ArtCraft\PhotoCraft", &stub);
    // A decoy on PATH: the only CLI a PATH-only lookup finds, which is what the launcher did.
    std::fs::create_dir_all(format!(r"{dir}\fake-path")).unwrap();
    std::fs::copy(&stub, format!(r"{dir}\fake-path\photocraft-cli.exe")).unwrap();
    let input = format!(r"{dir}\a.png");
    let wanted = format!("droplet\n{}\\Rotate.pcdroplet\n{input}", dir);

    let (captured, report) = run_lookup_probe(
        &launcher,
        &input,
        Some(&format!(r"{dir}\fake-local")),
        Some(&format!(r"{dir}\fake-progfiles")),
        None,
        Some(&format!(r"{dir}\fake-path")),
        None,
    );
    let lines: Vec<&str> = captured.split('\n').collect();
    assert_eq!(lines.len(), 4, "one CLI must run and get 3 arguments; it recorded {captured:?}, cmd said: {report}");
    assert!(
        lines[0].ends_with(r"fake-local\Programs\ArtCraft\PhotoCraft\photocraft-cli.exe"),
        "the per-user install folder beats the per-machine one and PATH; it picked {captured:?}"
    );
    assert_eq!(lines[1..].join("\n"), wanted, "the droplet and the dropped path reach the picked CLI");

    // PHOTOCRAFT_CLI keeps the first place even when both install folders hold a CLI.
    let (captured, report) = run_lookup_probe(
        &launcher,
        &input,
        Some(&format!(r"{dir}\fake-local")),
        Some(&format!(r"{dir}\fake-progfiles")),
        None,
        Some(&format!(r"{dir}\fake-path")),
        Some(&stub),
    );
    assert!(captured.split('\n').next().unwrap_or_default().ends_with(r"cli who.exe"), "PHOTOCRAFT_CLI still wins; it picked {captured:?}, cmd said: {report}");
}

#[cfg(windows)]
#[test]
fn windows_launcher_falls_back_to_the_per_machine_install_folder() {
    // Per-user install folder exists but holds no CLI: the per-machine Program Files folder
    // is tried before PATH.
    let dir = tmp("install-lookup-machine");
    let stub = cli_stub_who(&dir);
    create(&dir, format!("{dir}/Rotate"), None);
    let launcher = format!(r"{dir}\Rotate.cmd");
    std::fs::write(format!(r"{dir}\a.png"), "x").unwrap();
    std::fs::create_dir_all(format!(r"{dir}\fake-local")).unwrap();
    install_stub(&format!(r"{dir}\fake-progfiles"), r"ArtCraft\PhotoCraft", &stub);
    std::fs::create_dir_all(format!(r"{dir}\fake-path")).unwrap();
    std::fs::copy(&stub, format!(r"{dir}\fake-path\photocraft-cli.exe")).unwrap();
    let input = format!(r"{dir}\a.png");
    let wanted = format!("droplet\n{}\\Rotate.pcdroplet\n{input}", dir);

    let (captured, report) = run_lookup_probe(
        &launcher,
        &input,
        Some(&format!(r"{dir}\fake-local")),
        Some(&format!(r"{dir}\fake-progfiles")),
        None,
        Some(&format!(r"{dir}\fake-path")),
        None,
    );
    let lines: Vec<&str> = captured.split('\n').collect();
    assert_eq!(lines.len(), 4, "one CLI must run and get 3 arguments; it recorded {captured:?}, cmd said: {report}");
    assert!(lines[0].ends_with(r"fake-progfiles\ArtCraft\PhotoCraft\photocraft-cli.exe"), "the per-machine install folder beats PATH; it picked {captured:?}");
    assert_eq!(lines[1..].join("\n"), wanted, "the droplet and the dropped path reach the picked CLI");
}

#[cfg(windows)]
#[test]
fn windows_launcher_falls_back_to_the_x86_per_machine_install_folder() {
    // The 64-bit MSI's default per-machine folder is `%ProgramFiles(x86)%\ArtCraft\PhotoCraft`
    // (packaging/windows/install-policy.wxi: CraftDefaultMachineFolder on VersionNT64), and a
    // native 64-bit cmd.exe resolves `%ProgramFiles%` to the 64-bit folder, so that install is
    // only found when both machine folders are searched (#13718, review thread 4637631427).
    // Red at f81bf94ca: with only `%ProgramFiles%` searched the launcher fell through to the
    // decoy CLI on PATH, which is where a drop on such an install failed.
    let dir = tmp("install-lookup-x86");
    let stub = cli_stub_who(&dir);
    create(&dir, format!("{dir}/Rotate"), None);
    let launcher = format!(r"{dir}\Rotate.cmd");
    std::fs::write(format!(r"{dir}\a.png"), "x").unwrap();
    std::fs::create_dir_all(format!(r"{dir}\fake-local")).unwrap();
    std::fs::create_dir_all(format!(r"{dir}\fake-progfiles")).unwrap();
    install_stub(&format!(r"{dir}\fake-x86"), r"ArtCraft\PhotoCraft", &stub);
    std::fs::create_dir_all(format!(r"{dir}\fake-path")).unwrap();
    std::fs::copy(&stub, format!(r"{dir}\fake-path\photocraft-cli.exe")).unwrap();
    let input = format!(r"{dir}\a.png");
    let wanted = format!("droplet\n{}\\Rotate.pcdroplet\n{input}", dir);

    let (captured, report) = run_lookup_probe(
        &launcher,
        &input,
        Some(&format!(r"{dir}\fake-local")),
        Some(&format!(r"{dir}\fake-progfiles")),
        Some(&format!(r"{dir}\fake-x86")),
        Some(&format!(r"{dir}\fake-path")),
        None,
    );
    let lines: Vec<&str> = captured.split('\n').collect();
    assert_eq!(lines.len(), 4, "one CLI must run and get 3 arguments; it recorded {captured:?}, cmd said: {report}");
    assert!(lines[0].ends_with(r"fake-x86\ArtCraft\PhotoCraft\photocraft-cli.exe"), "the x86 per-machine install folder beats PATH; it picked {captured:?}");
    assert_eq!(lines[1..].join("\n"), wanted, "the droplet and the dropped path reach the picked CLI");

    // Both machine folders hold a CLI: the 64-bit one keeps its place, so the lookup order is
    // PHOTOCRAFT_CLI, per-user, per-machine 64-bit, per-machine x86, PATH.
    install_stub(&format!(r"{dir}\fake-progfiles"), r"ArtCraft\PhotoCraft", &stub);
    let (captured, report) = run_lookup_probe(
        &launcher,
        &input,
        Some(&format!(r"{dir}\fake-local")),
        Some(&format!(r"{dir}\fake-progfiles")),
        Some(&format!(r"{dir}\fake-x86")),
        Some(&format!(r"{dir}\fake-path")),
        None,
    );
    let lines: Vec<&str> = captured.split('\n').collect();
    assert_eq!(lines.len(), 4, "one CLI must run and get 3 arguments; it recorded {captured:?}, cmd said: {report}");
    assert!(
        lines[0].ends_with(r"fake-progfiles\ArtCraft\PhotoCraft\photocraft-cli.exe"),
        "the 64-bit per-machine folder is searched before the x86 one; it picked {captured:?}"
    );
    assert_eq!(lines[1..].join("\n"), wanted, "the droplet and the dropped path reach the picked CLI");
}

#[cfg(windows)]
#[test]
fn windows_launcher_splits_an_unquoted_dropped_path() {
    // Written as the byte-exact expectation first: red on 2b269d479, the CLI recorded
    // `...\pics\a` (the `&` split the line) and `...\pics\xy.png` (cmd ate the caret).
    // File Explorer quotes a dropped path only when it contains a space, so the unquoted forms
    // below reach cmd.exe exactly as Explorer's command line for a drop does. cmd splits and
    // unescapes that line while it PARSES it, before the launcher's first line runs, so no
    // batch body can recover the path. Tried: a shift loop that re-quotes each `%1` (it forwards
    // the same cut argument, and its delayed expansion also swallows everything between two `!`
    // of a path like `po!p.png`, which the bare `%*` form passes today), and a `%CMDCMDLINE%`
    // rewrite (copying that variable splits the line a second time and yields merged tokens).
    // A `.lnk` launcher - Explorer then hands the paths straight to the exe - is the way out,
    // and writing a Shell Link is a decision for the maintainers.
    let dir = tmp("unquoted-drop");
    if dir.contains(' ') {
        // The probe feeds the dropped paths to cmd UNQUOTED, exactly as Explorer's command line
        // for a drop without a space does. If the scratch path itself contains a space, cmd cuts
        // that path at the space before `&` or `^` ever reach the parser, and the recorded
        // arguments would pin the scratch location instead of the parser behaviour. Skip with the
        // reason on such a machine; drops that carry a space stay pinned, quoted, by
        // windows_launcher_runs_the_cli_on_every_dropped_path.
        eprintln!("skipping the unquoted-path probe: the scratch path contains a space ({dir})");
        return;
    }
    let stub = cli_stub(&dir);
    let pics = format!(r"{dir}\pics");
    std::fs::create_dir_all(&pics).unwrap();
    create(&dir, format!("{dir}/Rotate"), None);
    let launcher = format!(r"{dir}\Rotate.cmd");
    let ampersand = format!(r"{pics}\a&b.png");
    let caret = format!(r"{pics}\x^y.png");
    std::fs::write(&ampersand, "x").unwrap();
    std::fs::write(&caret, "x").unwrap();
    let captured = format!(r"{dir}\captured.txt");
    let drop = |line: &str| -> (Vec<String>, String) {
        use std::os::windows::process::CommandExt;
        let _ = std::fs::remove_file(&captured);
        let output = std::process::Command::new("cmd.exe")
            .raw_arg(format!("/d /c \"\"{launcher}\" {line}\""))
            .env("PHOTOCRAFT_CLI", &stub)
            .env("STUB_OUT", &captured)
            .output()
            .unwrap();
        let text = std::fs::read_to_string(&captured).unwrap_or_default();
        (text.split('\n').map(str::to_string).collect(), String::from_utf8_lossy(&output.stderr).into_owned())
    };

    // `&`: the CLI is called with the part before the ampersand, and cmd runs the rest as a
    // second command. The path of the dropped file never arrives.
    let (lines, stderr) = drop(&ampersand);
    assert_eq!(lines.len(), 3, "an unquoted & drop reaches the CLI as ONE truncated argument, cmd stderr: {stderr}");
    assert_eq!(lines[0], "droplet", "the launcher still calls the CLI's droplet subcommand");
    assert_eq!(lines[1], format!(r"{dir}\Rotate.pcdroplet"), "the launcher's own droplet self-location is unaffected");
    assert_eq!(lines[2], format!(r"{pics}\a"), "cmd cut the dropped path at the &");
    assert!(stderr.contains("'b.png' is not recognized"), "cmd treated the rest of the path as a command: {stderr}");

    // `^`: one argument reaches the CLI, with the caret removed by the parser.
    let (lines, stderr) = drop(&caret);
    assert_eq!(lines.len(), 3, "cmd ate the caret of an unquoted x^y.png, cmd stderr: {stderr}");
    assert_eq!(lines[2], format!(r"{pics}\xy.png"), "the path arrives unescaped, so the CLI cannot find it");
}
