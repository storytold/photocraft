//! Batch's Destination (#13659): None, Save and Close and Folder.
//!
//! Measured in the reference application with an action that inverts the active layer, over a
//! folder holding a PNG, a JPEG and a two-layer PSD:
//!
//! * `destination: "none"` - every processed file stays open, changed and unsaved; nothing is written.
//! * `destination: "saveAndClose"` - each file is written over itself in its own format (#10026).
//! * `destination: "folder"`, and a request that names no destination at all - the copy run that
//!   writes each result into `output`, which every existing caller depends on.

use super::*;

fn destination_dir(name: &str) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("photocraft-batch-destination-{}-{name}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// A solid-colour PNG written to `dir`, and its path.
fn source_png(dir: &str, name: &str, color: &str) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2, "height": 1, "background": color})).unwrap();
    let path = join(dir, name);
    save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    path
}

fn top_left_pixel(doc: &Document) -> [f32; 4] {
    photocraft_compose::render(doc, Rect::new(0, 0, 1, 1)).px[0]
}

/// Destination None with a folder source opens each file and plays the action on it: the
/// reference application shows every file on a tab, changed, with nothing written.
#[test]
fn batch_destination_none_opens_every_folder_file_and_writes_nothing() {
    let dir = destination_dir("none-open");
    let a = source_png(&dir, "a.png", "#ff0000");
    let b = source_png(&dir, "b.png", "#00ff00");
    let bytes_a = std::fs::read(&a).unwrap();
    let bytes_b = std::fs::read(&b).unwrap();
    let mut s = Session::new();
    let result =
        s.execute("file.automate.batch", json!({"input": [a.clone(), b.clone()], "destination": "none", "steps": [["image.adjustments.invert", {}]]})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([]), "None writes nothing: {result}");
    let docs = s.documents();
    assert_eq!(docs.len(), 2, "both files stayed open");
    assert_eq!(docs[0].path.as_deref(), Some(a.as_str()));
    assert_eq!(docs[1].path.as_deref(), Some(b.as_str()));
    assert_eq!(top_left_pixel(&docs[0].doc), [0.0, 1.0, 1.0, 1.0], "a.png: the action ran on it");
    assert_eq!(top_left_pixel(&docs[1].doc), [1.0, 0.0, 1.0, 1.0], "b.png: the action ran on it");
    assert_eq!(std::fs::read(&a).unwrap(), bytes_a, "the source file is untouched");
    assert_eq!(std::fs::read(&b).unwrap(), bytes_b, "the source file is untouched");
    assert!(std::fs::read_dir(&dir).unwrap().count() == 2, "and no extra file appeared: {result}");
}

/// The opened file reads as an ordinary edited tab: the changed mark is on it, the run sits in
/// its history as one state, and Undo takes the step back without touching the file.
#[test]
fn batch_destination_none_leaves_each_opened_file_changed_with_the_step_in_its_history() {
    let dir = destination_dir("none-changed");
    let a = source_png(&dir, "a.png", "#ff0000");
    let bytes = std::fs::read(&a).unwrap();
    let mut s = Session::new();
    s.execute("file.automate.batch", json!({"input": [a.clone()], "destination": "none", "steps": [["image.adjustments.invert", {}]]})).unwrap();
    {
        let st = s.active().unwrap();
        assert!(st.is_dirty(), "the tab shows the changed mark");
        assert_eq!(st.history.past_len(), 1, "one history state for the run");
        // History › "Create a single history state for actions and batching" is on, so the steps
        // read as one state named for the run, as a played action's do (#2752).
        assert_eq!(st.history.undo_label(), Some("Action: Batch"), "{}", st.history.undo_label().unwrap_or_default());
    }
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(top_left_pixel(&s.active().unwrap().doc), [1.0, 0.0, 0.0, 1.0], "Undo takes the step back");
    assert!(s.active().unwrap().is_dirty(), "Undo is a step of its own, so the tab keeps its changed mark");
    assert_eq!(std::fs::read(&a).unwrap(), bytes, "Undo wrote nothing either");
}

/// An action that carries its own Save step still writes nothing with Destination None; the
/// document keeps its changed mark, as it does for Opened Files without an output folder.
#[test]
fn batch_destination_none_with_an_action_save_step_writes_no_file() {
    let dir = destination_dir("none-save-step");
    let a = source_png(&dir, "a.png", "#ff0000");
    let bytes = std::fs::read(&a).unwrap();
    let mut s = Session::new();
    let result = s
        .execute("file.automate.batch", json!({"input": [a.clone()], "destination": "none", "steps": [["image.adjustments.invert", {}], ["file.saveAs", {}]]}))
        .unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([]), "the recorded Save writes no file: {result}");
    assert_eq!(std::fs::read(&a).unwrap(), bytes, "the source is untouched");
    let st = s.active().unwrap();
    assert_eq!(top_left_pixel(&st.doc), [0.0, 1.0, 1.0, 1.0]);
    assert!(st.is_dirty(), "a Save with no destination leaves the document changed");
}

/// Destination None with Source: Opened Files keeps that source's behaviour: the open documents
/// are played and nothing is written for them.
#[test]
fn batch_destination_none_on_opened_files_writes_nothing_and_leaves_them_changed() {
    let dir = destination_dir("none-open-source");
    let a = source_png(&dir, "a.png", "#ff0000");
    let mut s = Session::new();
    open_bytes_as(&mut s, "a.png", &std::fs::read(&a).unwrap(), None, Some(a.clone())).unwrap();
    let bytes = std::fs::read(&a).unwrap();
    let result = s
        .execute("file.automate.batch", json!({"source": "open", "destination": "none", "input": [a.clone()], "steps": [["image.adjustments.invert", {}]]}))
        .unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([]), "opened files are played, not written: {result}");
    assert_eq!(result["documents"], json!(1), "{result}");
    assert_eq!(top_left_pixel(&s.active().unwrap().doc), [0.0, 1.0, 1.0, 1.0]);
    assert!(s.active().unwrap().is_dirty());
    assert_eq!(std::fs::read(&a).unwrap(), bytes, "still nothing written");
}

/// Destination None opens the sources itself: the options that only the scratch folder run has a
/// meaning for are refused instead of silently doing something else.
#[test]
fn batch_destination_none_refuses_the_recorded_folder_open_options() {
    let dir = destination_dir("none-recorded-options");
    let a = source_png(&dir, "a.png", "#ff0000");
    let mut s = Session::new();
    for extra in [json!({"overrideOpen": true}), json!({"suppressOpenOptions": true}), json!({"suppressWarnings": true}), json!({"includeNonImages": true})] {
        let mut p = json!({"input": [a.clone()], "destination": "none", "steps": []});
        for (key, value) in extra.as_object().unwrap() {
            p[key] = value.clone();
        }
        let error = s.execute("file.automate.batch", p).unwrap_err().to_string();
        assert!(error.contains("destination"), "{error}");
        assert_eq!(s.documents().len(), 0, "a refused run opens nothing: {error}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "and writes nothing: {error}");
    }
}

/// Destination None writes nothing, so an output folder would be a contradiction: the command
/// refuses it rather than picking a destination out of the request.
#[test]
fn batch_destination_none_refuses_an_output_folder() {
    let dir = destination_dir("none-output");
    let a = source_png(&dir, "a.png", "#ff0000");
    let out = join(&dir, "out");
    let mut s = Session::new();
    let error = s.execute("file.automate.batch", json!({"input": [a.clone()], "destination": "none", "output": out, "steps": []})).unwrap_err().to_string();
    assert!(error.contains("output"), "{error}");
    assert_eq!(s.documents().len(), 0, "a refused run opens nothing");
    assert!(!std::path::Path::new(&out).exists(), "and creates no output folder");
}

/// A blank destination from a form is no destination: the folder run stays the default, so every
/// caller that names none keeps writing its copies.
#[test]
fn batch_destination_null_keeps_the_folder_default() {
    let dir = destination_dir("null-default");
    let a = source_png(&dir, "a.png", "#ff0000");
    let out = join(&dir, "out");
    std::fs::create_dir_all(&out).unwrap();
    let mut s = Session::new();
    let result = s.execute("file.automate.batch", json!({"input": [a], "destination": null, "output": out, "steps": []})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([join(&out, "a.png")]), "{result}");
    assert_eq!(s.documents().len(), 0, "the folder run opens no tabs");
}

/// Back-compat (#13659 does not move the engine's default): without `destination` the folder run
/// still needs its `output`, and without it still answers BadParams instead of opening files.
#[test]
fn batch_without_a_destination_still_requires_an_output_folder() {
    let dir = destination_dir("no-destination");
    let a = source_png(&dir, "a.png", "#ff0000");
    let out = join(&dir, "out");
    std::fs::create_dir_all(&out).unwrap();
    let mut s = Session::new();
    let written = s.execute("file.automate.batch", json!({"input": [a.clone()], "output": out.clone(), "steps": []})).unwrap();
    assert_eq!(written["files"], json!([join(&out, "a.png")]), "{written}");
    assert_eq!(s.documents().len(), 0, "the copy run opened nothing");
    let error = s.execute("file.automate.batch", json!({"input": [a], "steps": []})).unwrap_err().to_string();
    assert!(error.contains("output"), "{error}");
    assert_eq!(s.documents().len(), 0, "and the refused run opened nothing either");
}

/// Destination "folder" spelled out is today's run: a copy in `output`, the sources unchanged,
/// and no document added to the session.
#[test]
fn batch_destination_folder_copies_and_opens_no_documents() {
    let dir = destination_dir("explicit-folder");
    let a = source_png(&dir, "a.png", "#ff0000");
    let bytes = std::fs::read(&a).unwrap();
    let out = join(&dir, "out");
    std::fs::create_dir_all(&out).unwrap();
    let mut s = Session::new();
    let result = s
        .execute("file.automate.batch", json!({"input": [a.clone()], "destination": "folder", "output": out, "steps": [["image.adjustments.invert", {}]]}))
        .unwrap();
    assert_eq!(result["files"], json!([join(&out, "a.png")]), "{result}");
    assert_eq!(std::fs::read(&a).unwrap(), bytes, "the source keeps its pixels");
    let copy = photocraft_io::import("copy.png", &std::fs::read(join(&out, "a.png")).unwrap()).unwrap().document;
    assert_eq!(top_left_pixel(&copy), [0.0, 1.0, 1.0, 1.0], "the copy holds the action");
    assert_eq!(s.documents().len(), 0, "Folder opens no tabs");
}

/// Destination "saveAndClose" overwrites each source in its own format and keeps the file open
/// nowhere: that is the half of the dropdown that already worked, pinned for the new default.
#[test]
fn batch_destination_save_and_close_still_overwrites_the_source() {
    let dir = destination_dir("save-and-close");
    let a = source_png(&dir, "a.png", "#ff0000");
    let bytes = std::fs::read(&a).unwrap();
    let mut s = Session::new();
    let result =
        s.execute("file.automate.batch", json!({"input": [a.clone()], "destination": "saveAndClose", "steps": [["image.adjustments.invert", {}]]})).unwrap();
    assert_eq!(result["files"], json!([a]), "{result}");
    assert_ne!(std::fs::read(&a).unwrap(), bytes, "the source was rewritten in place");
    let saved = photocraft_io::import("a.png", &std::fs::read(&a).unwrap()).unwrap().document;
    assert_eq!(top_left_pixel(&saved), [0.0, 1.0, 1.0, 1.0], "and in its own format");
    assert_eq!(s.documents().len(), 0, "Save and Close leaves no tab open");
}

/// Anything else is still a parameter error: the dropdown's three values are the whole set.
#[test]
fn batch_rejects_a_destination_that_is_not_none_folder_or_save_and_close() {
    let dir = destination_dir("bad-destination");
    let a = source_png(&dir, "a.png", "#ff0000");
    let mut s = Session::new();
    for bad in ["open", "None", "saveAndClose ", "inPlace"] {
        let error = s.execute("file.automate.batch", json!({"input": [a.clone()], "destination": bad, "steps": []})).unwrap_err().to_string();
        assert!(error.contains("destination"), "{bad}: {error}");
    }
    assert_eq!(s.documents().len(), 0);
}

/// An input that will not open is reported as a played failure is: in the answer's errors and in
/// the Log to File the run writes, while the input that did open stays on a tab, played.
#[test]
fn batch_destination_none_lists_an_unreadable_input_in_its_error_log() {
    let dir = destination_dir("none-error-log");
    let good = source_png(&dir, "good.png", "#ff0000");
    let corrupt = join(&dir, "corrupt.png");
    std::fs::write(&corrupt, b"not a PNG").unwrap();
    let log = join(&dir, "batch.log");
    let mut s = Session::new();
    let result = s
        .execute(
            "file.automate.batch",
            json!({"input": [corrupt.clone(), good.clone()], "destination": "none", "errorLog": log.clone(), "steps": [["image.adjustments.invert", {}]]}),
        )
        .unwrap();
    assert_eq!(result["errors"].as_array().unwrap().len(), 1, "{result}");
    assert_eq!(result["errors"][0]["file"], json!(corrupt), "{result}");
    assert_eq!(result["files"], json!([]), "None writes no result: {result}");
    assert_eq!(result["documents"], json!(1), "the readable input still ran: {result}");
    assert_eq!(s.documents().len(), 1, "and stayed open");
    assert_eq!(top_left_pixel(&s.active().unwrap().doc), [0.0, 1.0, 1.0, 1.0]);
    // Photoshop's Batch log encoding, as main's format test pins it: UTF-16 BE with a BOM, one File/Error pair per failure.
    let bytes = std::fs::read(&log).unwrap();
    assert!(bytes.starts_with(&[0xfe, 0xff]), "the log keeps its UTF-16 BE BOM");
    let words: Vec<u16> = bytes[2..].chunks(2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect();
    let text = String::from_utf16(&words).unwrap();
    // The log names paths with the platform separators, as main's format test pins it; the run's input keeps them as given.
    let logged = |path: &str| format!("File: \"{}\"", path.replace('/', std::path::MAIN_SEPARATOR_STR));
    assert!(text.contains(&logged(&corrupt)), "the open failure is in the log: {text}");
    assert!(!text.contains(&logged(&good)), "the file that opened is not an error: {text}");
}

/// A folder source with no eligible image is an empty run, not a Source: Opened Files run: the
/// action must not play on the tabs the user already had open (#13659 review).
#[test]
fn batch_destination_none_on_an_empty_folder_runs_nothing_on_the_open_tabs() {
    let dir = destination_dir("empty-folder");
    let empty = join(&dir, "empty");
    std::fs::create_dir_all(&empty).unwrap();
    let a = source_png(&dir, "a.png", "#ff0000");
    let bytes = std::fs::read(&a).unwrap();
    let mut s = Session::new();
    open_bytes_as(&mut s, "a.png", &bytes, None, Some(a.clone())).unwrap();
    let result = s.execute("file.automate.batch", json!({"input": empty, "destination": "none", "steps": [["image.adjustments.invert", {}]]})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["documents"], json!(0), "an empty folder has no input to play: {result}");
    assert_eq!(s.documents().len(), 1, "the run added no tabs");
    assert_eq!(top_left_pixel(&s.active().unwrap().doc), [1.0, 0.0, 0.0, 1.0], "the already-open tab was not played");
    assert!(!s.active().unwrap().is_dirty(), "and not marked changed");
    assert_eq!(std::fs::read(&a).unwrap(), bytes, "and nothing was written");
}

/// The error log must not replace any input, including one that fails to open and so never
/// reaches the guard over the opened files (#13659 review): the run is refused before a tab opens.
#[test]
fn batch_destination_none_refuses_an_error_log_that_names_one_of_the_inputs() {
    let dir = destination_dir("error-log-input");
    let corrupt = join(&dir, "corrupt.png");
    std::fs::write(&corrupt, b"not a PNG").unwrap();
    let good = source_png(&dir, "good.png", "#ff0000");
    let mut s = Session::new();
    // The log names the input that will not open: writing it would replace that file.
    let error = s
        .execute("file.automate.batch", json!({"input": [corrupt.clone(), good.clone()], "destination": "none", "errorLog": corrupt.clone(), "steps": []}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("errorLog"), "{error}");
    assert_eq!(s.documents().len(), 0, "the refused run opens no tab at all: {error}");
    assert_eq!(std::fs::read(&corrupt).unwrap(), b"not a PNG", "the log did not replace the input");
    // And the same guard now runs before opening, for a log naming an input that would open.
    let error = s
        .execute("file.automate.batch", json!({"input": [corrupt.clone(), good.clone()], "destination": "none", "errorLog": good.clone(), "steps": []}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("errorLog"), "{error}");
    assert_eq!(s.documents().len(), 0, "the second refused run opens no tab either: {error}");
}

/// The dialog can now ask for Source: Opened Files beside Save and Close. The issue measured no
/// such combination, and the command keeps its existing rule: Save and Close needs a folder source.
#[test]
fn batch_destination_save_and_close_still_refuses_an_opened_files_source() {
    let dir = destination_dir("save-close-open-source");
    let a = source_png(&dir, "a.png", "#ff0000");
    let bytes = std::fs::read(&a).unwrap();
    let mut s = Session::new();
    open_bytes_as(&mut s, "a.png", &bytes, None, Some(a.clone())).unwrap();
    let error = s.execute("file.automate.batch", json!({"source": "open", "destination": "saveAndClose", "steps": []})).unwrap_err().to_string();
    assert!(error.contains("require a folder source"), "{error}");
    assert_eq!(std::fs::read(&a).unwrap(), bytes, "the refused run wrote nothing");
}
