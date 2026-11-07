use super::*;

#[cfg(unix)]
fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("photocraft-raw-editor-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn with_editor(path: &str) -> Session {
    let mut s = Session::new();
    let editor = path.to_string();
    s.edit_prefs(|p| p.raw_defaults.external_editor = editor);
    s
}

#[test]
fn only_darktable_and_rapidraw_are_editors() {
    let dt = editor_paths("/usr/bin/darktable").unwrap();
    assert_eq!((dt.editor, dt.cli), (Editor::Darktable, PathBuf::from("/usr/bin/darktable-cli")));
    let cli = editor_paths("/opt/dt/bin/darktable-cli").unwrap();
    assert_eq!((cli.gui, cli.cli), (PathBuf::from("/opt/dt/bin/darktable"), PathBuf::from("/opt/dt/bin/darktable-cli")));
    let win = editor_paths("/opt/darktable/bin/Darktable.exe").unwrap();
    assert_eq!(win.cli, PathBuf::from("/opt/darktable/bin/darktable-cli.exe"));
    let mac = editor_paths("/Applications/darktable.app").unwrap();
    assert_eq!(mac.cli, PathBuf::from("/Applications/darktable.app/Contents/MacOS/darktable-cli"));
    let rr = editor_paths("/usr/bin/rapidraw").unwrap();
    assert_eq!((rr.editor, &rr.gui, &rr.cli), (Editor::RapidRaw, &rr.cli, &PathBuf::from("/usr/bin/rapidraw")));
    assert_eq!(editor_paths("/Applications/RapidRAW.app").unwrap().cli, PathBuf::from("/Applications/RapidRAW.app/Contents/MacOS/RapidRAW"));
    // Any other program is refused, as is nothing at all.
    for p in ["", "  ", "/bin/sh", "/usr/bin/darktable-evil", "darktable.sh", "/tmp/rapidraw/x"] {
        assert!(editor_paths(p).is_err(), "{p}");
    }
}

#[test]
fn commands_follow_each_editors_manual() {
    let dt = editor_paths("/usr/bin/darktable").unwrap();
    assert_eq!(edit_command(&dt, "/p/a.cr2"), ("/usr/bin/darktable".into(), vec!["/p/a.cr2".to_string()]));
    let (program, args) = develop_command(&dt, "/p/a.cr2", "/t/a.tif");
    // A path, not a string: Windows joins the sibling with `\`.
    assert_eq!(Path::new(&program), Path::new("/usr/bin/darktable-cli"));
    assert_eq!(args, ["/p/a.cr2", "/t/a.tif", "--apply-custom-presets", "false", "--core", "--conf", "plugins/imageio/format/tiff/bpp=16"]);
    let rr = editor_paths("/usr/bin/RapidRAW").unwrap();
    let (program, args) = develop_command(&rr, "/p/a.cr2", "/t/a.tif");
    assert_eq!(program, "/usr/bin/RapidRAW");
    assert_eq!(args, ["export", "/p/a.cr2", "--output", "/t/a.tif", "--format", "tiff", "--tiff-bit-depth", "16"]);
    let mac = editor_paths("/Applications/RapidRAW.app").unwrap();
    assert_eq!(edit_command(&mac, "/p/a.nef"), ("open".into(), vec!["-a".to_string(), "/Applications/RapidRAW.app".into(), "/p/a.nef".into()]));
}

#[test]
fn bad_params_and_settings_are_errors() {
    let mut s = Session::new();
    // No editor set.
    assert!(s.execute(EDIT, json!({"path": "/p/a.cr2", "dryRun": true})).is_err());
    let mut s2 = with_editor("/bin/sh");
    assert!(s2.execute(DEVELOP, json!({"path": "/p/a.cr2", "dryRun": true})).is_err());
    let mut s = with_editor("/usr/bin/darktable");
    for p in [json!({}), json!({"path": ""}), json!({"path": 3}), json!({"path": "/p/a.png"}), json!({"path": "/p/noext"})] {
        assert!(s.execute(EDIT, p.clone()).is_err(), "{p}");
        assert!(s.execute(DEVELOP, p.clone()).is_err(), "{p}");
    }
    // A raw that isn't there.
    assert!(s.execute(EDIT, json!({"path": "/no/such/a.cr2"})).is_err());
    assert!(s.execute(DEVELOP, json!({"path": "/no/such/a.cr2"})).is_err());
}

#[test]
fn a_relative_raw_is_made_absolute() {
    let mut s = with_editor("/usr/bin/darktable");
    let r = s.execute(EDIT, json!({"path": "-shoot.cr2", "dryRun": true})).unwrap();
    let raw = r["args"][0].as_str().unwrap();
    assert!(std::path::Path::new(raw).is_absolute() && !raw.starts_with('-'), "{raw}");
}

/// A stand-in `darktable-cli` in `dir`: a link to `/bin/sh`, which runs the "raw" (its first
/// argument) as a script, with the output path as `$1`. No test executes a file it just wrote:
/// another test thread's fork could still hold it open for writing (ETXTBSY).
#[cfg(unix)]
fn stand_in_darktable(dir: &Path) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink("/bin/sh", bin.join("darktable-cli")).unwrap();
    bin.join("darktable")
}

/// A stand-in `darktable-cli` writes a TIFF where it is told; Develop opens it as a new, untitled
/// document named after the raw.
#[cfg(unix)]
#[test]
fn develop_opens_what_the_editor_writes() {
    let dir = tmp("develop");
    let tif = dir.join("made.tif");
    let mut src = Session::new();
    src.execute("file.new", json!({"width": 6, "height": 4, "background": "#336699"})).unwrap();
    crate::file_cmds::save_doc(&src.active().unwrap().doc, &tif.to_string_lossy(), None).unwrap();
    // Arguments after the raw: output, options. Records them, then writes the output.
    let script = format!("printf '%s\\n' \"$@\" > '{}'\ncp '{}' \"$1\"\n", dir.join("args.txt").display(), tif.display());
    let raw = dir.join("IMG_0001.CR2");
    std::fs::write(&raw, script).unwrap();
    let mut s = with_editor(&stand_in_darktable(&dir).to_string_lossy());
    let r = s.execute(DEVELOP, json!({"path": raw.to_string_lossy()})).unwrap();
    assert_eq!(r["name"], "IMG_0001.tif");
    let d = s.active().unwrap();
    assert_eq!((d.doc.size.width, d.doc.size.height), (6, 4));
    assert_eq!(d.doc.name, "IMG_0001.tif");
    assert_eq!(d.path, None, "a save asks where, not into the scratch folder");
    let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    assert!(args.starts_with("/") && args.contains("IMG_0001.tif\n--apply-custom-presets\n"), "{args}");
    assert!(args.contains("plugins/imageio/format/tiff/bpp=16"), "{args}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An editor that fails, or writes nothing, is an error naming it, and opens nothing.
#[cfg(unix)]
#[test]
fn a_failed_develop_opens_nothing() {
    let dir = tmp("fail");
    let raw = dir.join("a.nef");
    let editor = stand_in_darktable(&dir);
    for (body, want) in [("echo 'cannot read file' >&2\nexit 3", "cannot read file"), ("exit 0", "wrote no image")] {
        std::fs::write(&raw, format!("{body}\n")).unwrap();
        let mut s = with_editor(&editor.to_string_lossy());
        let e = s.execute(DEVELOP, json!({"path": raw.to_string_lossy()})).unwrap_err().to_string();
        assert!(e.contains(want), "{e}");
        assert!(s.documents().is_empty());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stuck editor is stopped at the deadline, and at once when the job is cancelled; both errors
/// name the editor.
#[cfg(unix)]
#[test]
fn a_stuck_editor_is_stopped() {
    let dir = tmp("stuck");
    // `exec`: the kill stops `sleep` itself, not a shell that leaves it running.
    let program = "/bin/sh";
    let args = ["-c".to_string(), "exec sleep 30".to_string()];
    let started = std::time::Instant::now();
    let ctx = crate::jobs::JobCtx::new();
    let e = run("darktable", program, &args, &dir, &ctx, std::time::Duration::from_millis(300)).unwrap_err().to_string();
    assert!(e.contains("darktable was stopped") && e.contains("longer than"), "{e}");
    let ctx = crate::jobs::JobCtx::new();
    ctx.cancel();
    let e = run("darktable", program, &args, &dir, &ctx, std::time::Duration::from_secs(600)).unwrap_err().to_string();
    assert!(e.contains("cancelled"), "{e}");
    // An editor that exits at once after a cancel opens nothing either.
    let quick = ["-c".to_string(), "exit 0".to_string()];
    let e = run("darktable", program, &quick, &dir, &ctx, std::time::Duration::from_secs(600)).unwrap_err().to_string();
    assert!(e.contains("cancelled"), "{e}");
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}
