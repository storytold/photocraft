//! Batch over Source: Opened Files (#13673). With a destination folder the reference application
//! writes each copy and closes that document; a document whose step or write failed stays open and
//! changed, and the documents the run never reached stay open too. Without a destination every
//! document stays open and changed, and a never-saved document keeps the headless rule of being
//! written as `<name>.psd` without asking.

use super::*;

fn tmp(name: &str) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("photocraft-batch-opened-close-{}-{name}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// Writes a solid-colour PNG of `w` x `h` and returns its path.
fn png(dir: &str, name: &str, w: u32, h: u32, color: &str) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": color})).unwrap();
    let path = join(dir, name);
    save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    path
}

/// Opens `paths` as live documents, in the given order.
fn open_as_documents(paths: &[String]) -> Session {
    let mut s = Session::new();
    for path in paths {
        s.execute("file.openAs", json!({"path": path})).unwrap();
    }
    s
}

/// The paths of the documents still open, in tab order.
fn open_paths(s: &Session) -> Vec<String> {
    s.documents().iter().map(|state| state.path.clone().unwrap_or_default()).collect()
}

#[test]
fn opened_files_with_a_folder_closes_each_document_after_its_file_is_written() {
    let dir = tmp("folder-closes-each");
    let first = png(&dir, "a.png", 8, 6, "#ff0000");
    let second = png(&dir, "b.png", 8, 6, "#0000ff");
    let output = join(&dir, "results");
    let mut s = open_as_documents(&[first, second]);
    let result =
        s.execute("file.automate.batch", json!({"source": "open", "steps": [["image.adjustments.invert", {}]], "output": output, "format": "png"})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([join(&output, "a.png"), join(&output, "b.png")]), "{result}");
    assert_eq!(result["documents"], 2, "both documents were run over");
    assert_eq!(s.documents().len(), 0, "each document closes once its copy is written");
    assert_eq!(s.active_index(), None);
}

#[test]
fn a_document_whose_write_failed_stays_open_and_changed_while_the_written_one_closes() {
    let dir = tmp("write-failed-stays-open");
    let first = png(&dir, "a.png", 8, 6, "#ff0000");
    let second = png(&dir, "b.png", 8, 6, "#0000ff");
    let output = join(&dir, "results");
    std::fs::create_dir_all(&output).unwrap();
    // A folder named `a.png` in the destination makes the write itself fail on every platform.
    std::fs::create_dir_all(join(&output, "a.png")).unwrap();
    let mut s = open_as_documents(&[first.clone(), second.clone()]);
    let result =
        s.execute("file.automate.batch", json!({"source": "open", "steps": [["image.adjustments.invert", {}]], "output": output, "format": "png"})).unwrap();
    assert_eq!(result["files"], json!([join(&output, "b.png")]), "{result}");
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    assert!(result["errors"][0]["document"].is_string(), "{result}");
    assert_eq!(open_paths(&s), vec![first], "only the document that failed stays open");
    assert!(s.documents()[0].is_dirty(), "and stays changed");
    assert_eq!(s.active_index(), Some(0));
}

#[test]
fn a_stop_for_errors_run_leaves_the_documents_it_never_reached_open() {
    let dir = tmp("stop-on-error-keeps-rest");
    let output = join(&dir, "results");
    std::fs::create_dir_all(&output).unwrap();
    // The second document's result lands on a folder, so its write fails.
    std::fs::create_dir_all(join(&output, "Second.png")).unwrap();
    let mut s = Session::new();
    for name in ["First", "Second", "Third"] {
        s.execute("file.new", json!({"name": name, "width": 8, "height": 6})).unwrap();
    }
    let result = s
        .execute(
            "file.automate.batch",
            json!({"source": "open", "steps": [["image.adjustments.invert", {}]], "output": output, "format": "png", "stopOnError": true}),
        )
        .unwrap();
    assert_eq!(result["files"], json!([join(&output, "First.png")]), "{result}");
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    assert_eq!(result["stopped"], json!("Second"), "{result}");
    assert_eq!(result["skipped"], json!(["Third"]), "{result}");
    let left: Vec<String> = s.documents().iter().map(|state| state.doc.name.clone()).collect();
    assert_eq!(left, ["Second", "Third"], "the failed document and the unreached one stay open");
    assert_eq!(result["errors"][0]["document"], json!("Second"), "{result}");
}

#[test]
fn a_never_saved_document_is_written_as_its_name_psd_and_closes() {
    let dir = tmp("never-saved-headless");
    let output = join(&dir, "results");
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "oracle", "width": 8, "height": 6})).unwrap();
    let result =
        s.execute("file.automate.batch", json!({"source": "open", "steps": [["image.adjustments.invert", {}]], "output": output, "format": "same"})).unwrap();
    assert_eq!(result["errors"], json!([]), "headless does not ask for a path");
    assert_eq!(result["files"], json!([join(&output, "oracle.psd")]), "{result}");
    assert!(std::path::Path::new(&join(&output, "oracle.psd")).is_file(), "{result}");
    assert_eq!(s.documents().len(), 0, "the written document closes like any other");
}

#[test]
fn opened_files_without_a_destination_stay_open_and_changed() {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "First", "width": 8, "height": 6})).unwrap();
    s.execute("file.new", json!({"name": "Second", "width": 12, "height": 5})).unwrap();
    let result = s.execute("file.automate.batch", json!({"source": "open", "steps": [["image.adjustments.invert", {}]]})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([]), "nothing is written");
    assert_eq!(result["documents"], 2);
    assert_eq!(s.documents().len(), 2, "Destination None leaves every document open");
    assert!(s.documents().iter().all(|state| state.is_dirty()), "and changed");
}

#[test]
fn a_folder_source_batch_still_leaves_the_sessions_open_documents() {
    let dir = tmp("folder-source-untouched");
    let input = png(&dir, "in.png", 8, 6, "#ff0000");
    let untouched = png(&dir, "keep.png", 6, 6, "#00ff00");
    let output = join(&dir, "out");
    let mut s = Session::new();
    s.execute("file.openAs", json!({"path": &untouched})).unwrap();
    let result =
        s.execute("file.automate.batch", json!({"input": [input], "output": output, "steps": [["image.adjustments.invert", {}]], "format": "png"})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([join(&output, "in.png")]), "{result}");
    assert_eq!(open_paths(&s), vec![untouched], "Source Folder closes nothing");
    assert!(!s.documents()[0].is_dirty(), "and does not edit the document open in the session");
}

#[test]
fn image_processor_on_opened_files_still_leaves_the_documents_open() {
    let dir = tmp("image-processor-open");
    let first = png(&dir, "a.png", 8, 6, "#ff0000");
    let output = join(&dir, "processed");
    let mut s = open_as_documents(std::slice::from_ref(&first));
    let result = s.execute("file.scripts.imageProcessor", json!({"source": "open", "output": output, "format": "png"})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"].as_array().map(Vec::len), Some(1), "{result}");
    assert_eq!(open_paths(&s), vec![first], "the Image Processor route is not part of #13673");
}

#[test]
fn a_step_that_closes_the_document_itself_keeps_todays_outcome() {
    let dir = tmp("step-closes-document");
    let first = png(&dir, "a.png", 8, 6, "#ff0000");
    let output = join(&dir, "out");
    let mut s = open_as_documents(&[first]);
    let result = s
        .execute("file.automate.batch", json!({"source": "open", "steps": [["file.close", { "save": "discard" }]], "output": output, "format": "png"}))
        .unwrap();
    // Not measured in the issue, so today's outcome stands: the closed document writes nothing.
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    assert_eq!(result["files"], json!([]), "{result}");
    assert!(s.documents().is_empty(), "closed by its own step, not closed a second time");
    assert!(!std::path::Path::new(&join(&output, "a.png")).exists());
}
