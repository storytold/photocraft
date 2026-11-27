//! Real-world community scripts, run end to end against a session as they were published. Each
//! fixture in `tests/it/community/` keeps its licence beside it (`<name>.LICENSE`) and its source in
//! `ATTRIBUTION.md`. The scripts are unchanged; what a test asserts is what PhotoCraft does with
//! them, including where a script reaches something PhotoCraft doesn't offer yet.

#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::rc::Rc;

use photocraft_engine::Session;
use photocraft_script_js::session_host::SessionHost;
use photocraft_script_js::{ModalKind, RunOptions, Runner, Step, Unlimited};
use serde_json::{Value, json};

/// What a run did: the dialogs it opened, in order, with their arguments, its output lines, and
/// how it ended.
#[derive(Debug)]
struct Run {
    dialogs: Vec<(ModalKind, Value)>,
    lines: Vec<String>,
    end: Result<(), String>,
}

/// Run `src` on `s`, answering each dialog with `answer(kind, args)`.
fn run(s: &Rc<RefCell<Session>>, name: &str, src: &str, mut answer: impl FnMut(ModalKind, &Value) -> Value) -> Run {
    let host = SessionHost::new(s.clone(), "community");
    let out = host.output();
    let options = RunOptions { script_name: Some(name.into()), ..RunOptions::default() };
    let mut r = Runner::new(src, Box::new(host), Box::new(Unlimited), options).unwrap();
    let mut dialogs = Vec::new();
    let end = loop {
        match r.step() {
            Step::Done => break Ok(()),
            Step::Paused(m) => {
                let reply = answer(m.kind, &m.args);
                dialogs.push((m.kind, m.args));
                assert!(dialogs.len() < 100, "a dialog loop: {dialogs:?}");
                r.answer(reply).unwrap();
            }
            Step::Failed(m) => break Err(m),
        }
    };
    let lines = out.borrow().clone();
    Run { dialogs, lines, end }
}

/// A document with a background, two layers and a group holding one more.
fn layered() -> Rc<RefCell<Session>> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "name": "Poster"})).unwrap();
    let mut add = |name: &str, x: i32, y: i32| {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        s.execute("select.rect", json!({"x": x, "y": y, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
    };
    add("Logo", 10, 20);
    add("Title", 40, 5);
    s.execute("layer.groupLayers", json!({"name": "Badges"})).unwrap();
    Rc::new(RefCell::new(s))
}

const COORDINATES: &str = include_str!("community/export_layer_coordinates.jsx");

/// Export Layer Coordinates asks for a folder, walks the layers (into groups) in pixels and
/// reports its export. Writing the text file is not offered to scripts yet: the script's own
/// `try` swallows that error, so it reports success without a file (see docs/scripting.md).
#[test]
fn export_layer_coordinates_runs_to_its_report() {
    let s = layered();
    // Not the default unit, so putting the units back is seen.
    s.borrow_mut().execute("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "inches"})).unwrap();
    let rulers = |s: &Rc<RefCell<Session>>| s.borrow_mut().execute("prefs.get", json!({"path": "unitsAndRulers.rulers"})).unwrap();
    let folder = if cfg!(windows) { r"C:\coords" } else { "/coords" };
    let at_picker = RefCell::new(Value::Null);
    let r = run(&s, "Export Layer Coordinates Photoshop.jsx", COORDINATES, |kind, _| match kind {
        ModalKind::SelectFolder => {
            // The script works in pixels: it has set the units by the time it asks.
            *at_picker.borrow_mut() = rulers(&s);
            json!(folder)
        }
        _ => Value::Null,
    });
    assert_eq!(r.end, Ok(()), "{r:?}");
    let kinds: Vec<ModalKind> = r.dialogs.iter().map(|(k, _)| *k).collect();
    assert_eq!(kinds, [ModalKind::SelectFolder, ModalKind::Alert], "{r:?}");
    assert_eq!(r.dialogs[0].1["prompt"], "Save exported coordinates to");
    let report = r.dialogs[1].1[0].as_str().unwrap_or_default();
    let feeds = if cfg!(windows) { "Windows" } else { "Macintosh" };
    // `Folder.toString()` gives the path with forward slashes.
    let dir = folder.replace('\\', "/");
    assert!(
        report.starts_with(&format!("Exported 2 layer's coordinates to {dir}/Poster.txt ")) && report.ends_with(&format!("using {feeds} line feeds.")),
        "{report}"
    );
    assert_eq!(r.dialogs[1].1[1], "Success!");
    assert!(r.lines.is_empty(), "{:?}", r.lines);
    // The script set pixels for its work and puts the units back as it found them.
    assert_eq!(*at_picker.borrow(), json!("pixels"));
    assert_eq!(rulers(&s), json!("inches"));
}

const EXPORT_FAST: &str = include_str!("community/export_layers_to_files_fast.jsx");

/// Export Layers To Files (Fast) gets through its setup (`app.stringIDToTypeID`,
/// `doc.suspendHistory`, `$.os`) to its `loadResource`, which checks that its dialog's resource
/// file exists next to the script before it opens it. The fixture comes without that file, so
/// the script stops at that check with its own alert: it never reaches `File.open` or reads the
/// resource.
#[test]
fn export_layers_to_files_fast_reaches_its_resource_file_check() {
    let s = layered();
    let r = run(&s, "Export Layers To Files (Fast).jsx", EXPORT_FAST, |_, _| Value::Null);
    assert_eq!(r.end, Ok(()), "{r:?}");
    assert_eq!(r.dialogs.len(), 1, "{r:?}");
    let (kind, args) = &r.dialogs[0];
    assert_eq!(*kind, ModalKind::Alert);
    assert_eq!(
        args[0].as_str(),
        Some(
            "Resource file 'Export%20Layers%20To%20Files%20(Fast)-progress_bar.json' for the export dialog is missing! Please, download the rest of the files that come with this script."
        ),
        "it got past its setup to the check"
    );
    assert_eq!(args[1].as_str(), Some("Error"));
}
