//! Batch over Source: Opened Files (#13673). With a destination folder the run writes a copy of
//! every opened document; the document itself then closes only when it has nothing unsaved and a
//! file of its own (the owner rule). A document with unsaved changes, a document that was never
//! saved, a document whose step or write failed and a document the run never reached all keep
//! their tabs, and without a destination nothing is written and nothing closes.

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

/// The names of the documents still open, in tab order.
fn open_names(s: &Session) -> Vec<String> {
    s.documents().iter().map(|state| state.doc.name.clone()).collect()
}

/// The top-left pixel of the file at `path`, as opening it again would show it.
fn top_left(path: &str) -> [f32; 4] {
    let imported = photocraft_io::import(&file_name(path), &std::fs::read(path).unwrap()).unwrap().document;
    photocraft_compose::render(&imported, Rect::new(0, 0, 1, 1)).px[0]
}

#[test]
fn opened_files_with_a_folder_closes_each_unchanged_document_after_its_file_is_written() {
    let dir = tmp("folder-closes-each");
    let first = png(&dir, "a.png", 8, 6, "#ff0000");
    let second = png(&dir, "b.png", 8, 6, "#0000ff");
    let output = join(&dir, "results");
    let mut s = open_as_documents(&[first, second]);
    let result = s.execute("file.automate.batch", json!({"source": "open", "steps": [], "output": output, "format": "png"})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([join(&output, "a.png"), join(&output, "b.png")]), "{result}");
    assert_eq!(result["documents"], 2, "both documents were run over");
    assert_eq!(s.documents().len(), 0, "each unchanged document closes once its copy is written");
    assert_eq!(s.active_index(), None);
}

#[test]
fn a_dirty_document_is_written_to_the_folder_and_stays_open_with_its_edits() {
    let dir = tmp("dirty-stays-open");
    let source = png(&dir, "a.png", 8, 6, "#ff0000");
    let output = join(&dir, "results");
    let before = std::fs::read(&source).unwrap();
    let mut s = open_as_documents(std::slice::from_ref(&source));
    let result =
        s.execute("file.automate.batch", json!({"source": "open", "steps": [["image.adjustments.invert", {}]], "output": output, "format": "png"})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([join(&output, "a.png")]), "the copy is still written");
    assert_eq!(open_paths(&s), vec![source.clone()], "the document with unsaved changes stays open");
    assert!(s.documents()[0].is_dirty(), "and is still marked changed");
    assert_eq!(s.active_index(), Some(0), "and stays the active tab");
    let live = photocraft_compose::render(&s.documents()[0].doc, Rect::new(0, 0, 1, 1)).px[0];
    assert_eq!(live, [0.0, 1.0, 1.0, 1.0], "and keeps the action's edit in the open document");
    assert_eq!(top_left(&join(&output, "a.png")), [0.0, 1.0, 1.0, 1.0], "the copy carries the action's edit");
    assert_eq!(top_left(&join(&dir, "a.png")), [1.0, 0.0, 0.0, 1.0], "the opened file keeps its own bytes");
    assert_eq!(std::fs::read(&source).unwrap(), before, "and its bytes exactly");
}

#[test]
fn a_never_saved_document_is_written_as_its_name_psd_and_stays_open() {
    let dir = tmp("never-saved-headless");
    let output = join(&dir, "results");
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "oracle", "width": 8, "height": 6})).unwrap();
    let result = s.execute("file.automate.batch", json!({"source": "open", "steps": [], "output": output, "format": "same"})).unwrap();
    assert_eq!(result["errors"], json!([]), "headless does not ask for a path");
    assert_eq!(result["files"], json!([join(&output, "oracle.psd")]), "{result}");
    assert!(std::path::Path::new(&join(&output, "oracle.psd")).is_file(), "{result}");
    assert_eq!(open_names(&s), ["oracle"], "a document with no file of its own stays open");
    assert_eq!(s.documents()[0].path, None, "and it has no file to be clean against");
    assert_eq!(s.active_index(), Some(0));
}

#[test]
fn a_run_with_no_steps_leaves_a_document_changed_before_the_run_open() {
    let dir = tmp("no-steps-dirty-stays");
    let source = png(&dir, "a.png", 8, 6, "#ff0000");
    let output = join(&dir, "results");
    let mut s = open_as_documents(std::slice::from_ref(&source));
    // Changed before the run, and the action has no step at all: nothing to discard, so the
    // document keeps its tab and its change while its copy is still written.
    s.execute("image.adjustments.invert", json!({})).unwrap();
    assert!(s.documents()[0].is_dirty(), "the run starts on a changed document");
    let result = s.execute("file.automate.batch", json!({"source": "open", "steps": [], "output": output, "format": "png"})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([join(&output, "a.png")]), "{result}");
    assert_eq!(open_paths(&s), vec![source.clone()], "the changed document stays open");
    assert!(s.documents()[0].is_dirty(), "with its change intact");
    assert_eq!(top_left(&join(&output, "a.png")), [0.0, 1.0, 1.0, 1.0], "the copy carries the change");
    assert_eq!(top_left(&join(&dir, "a.png")), [1.0, 0.0, 0.0, 1.0], "the opened file keeps its bytes");
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
    // The document whose write will fail carries the unsaved change; the other is unchanged, so it
    // closes with the run.
    s.set_active(0);
    s.execute("image.adjustments.invert", json!({})).unwrap();
    let result = s.execute("file.automate.batch", json!({"source": "open", "steps": [], "output": output, "format": "png"})).unwrap();
    assert_eq!(result["files"], json!([join(&output, "b.png")]), "{result}");
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    assert_eq!(result["errors"][0]["document"], json!("a.png"), "the error names the document that stays open");
    assert_eq!(open_paths(&s), vec![first], "only the document whose write failed stays open");
    assert!(s.documents()[0].is_dirty(), "and stays changed");
    assert_eq!(s.active_index(), Some(0));
}

#[test]
fn a_stop_for_errors_run_keeps_the_unreached_document_open_and_active() {
    let dir = tmp("stop-on-error-keeps-rest");
    let paths: Vec<String> = ["first.png", "second.png", "third.png"].iter().map(|name| png(&dir, name, 8, 6, "#ff0000")).collect();
    let output = join(&dir, "results");
    std::fs::create_dir_all(&output).unwrap();
    // The second document's result lands on a folder, so its write fails.
    std::fs::create_dir_all(join(&output, "second.png")).unwrap();
    let mut s = open_as_documents(&paths);
    let result = s.execute("file.automate.batch", json!({"source": "open", "steps": [], "output": output, "format": "png", "stopOnError": true})).unwrap();
    assert_eq!(result["files"], json!([join(&output, "first.png")]), "{result}");
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    assert_eq!(result["stopped"], json!("second.png"), "{result}");
    assert_eq!(result["skipped"], json!(["third.png"]), "{result}");
    assert_eq!(result["errors"][0]["document"], json!("second.png"), "{result}");
    assert_eq!(open_names(&s), ["second.png", "third.png"], "the failed document and the unreached one stay open");
    // "third.png" was the active tab when the run started: closing the written first document moved
    // it to index 1, and the run restores that tab instead of leaving the failed one active.
    assert_eq!(s.active_index(), Some(1), "the previously active tab is restored");
    assert_eq!(s.documents()[1].doc.name, "third.png", "and it is the document the run never reached");
}

#[test]
fn a_written_copy_never_rewrites_the_file_the_document_was_opened_from() {
    let dir = tmp("source-file-untouched");
    let source = png(&dir, "a.png", 8, 6, "#ff0000");
    let output = join(&dir, "results");
    let before = std::fs::read(&source).unwrap();
    let mut s = open_as_documents(std::slice::from_ref(&source));
    let result = s.execute("file.automate.batch", json!({"source": "open", "steps": [], "output": output, "format": "png"})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([join(&output, "a.png")]), "{result}");
    assert_eq!(s.documents().len(), 0, "the unchanged document closed with the run");
    assert_eq!(std::fs::read(&source).unwrap(), before, "the run wrote the copy only: the opened file keeps its bytes");
    assert_eq!(top_left(&source), [1.0, 0.0, 0.0, 1.0], "the opened file still decodes the original red");
    assert_eq!(top_left(&join(&output, "a.png")), [1.0, 0.0, 0.0, 1.0], "the copy is that same document");
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

#[test]
fn a_nested_opened_files_run_closes_the_tabs_the_outer_run_still_has_to_write() {
    let dir = tmp("nested-opened-files");
    let paths: Vec<String> = ["one.png", "two.png", "three.png"].iter().map(|name| png(&dir, name, 8, 6, "#ff0000")).collect();
    let inner = join(&dir, "inner");
    let outer = join(&dir, "outer");
    let mut s = open_as_documents(&paths);
    let result = s
        .execute(
            "file.automate.batch",
            json!({
                "source": "open", "output": outer, "format": "png",
                "steps": [["file.automate.batch", {"source": "open", "steps": [], "output": inner, "format": "png"}]]
            }),
        )
        .unwrap();
    // Not measured in the issue: the inner run wrote and closed every unchanged document, so the
    // outer run finds no document to write for any of its inputs. Every input reports it, and
    // nothing panics.
    assert_eq!(result["documents"], 3);
    assert_eq!(result["files"], json!([]), "{result}");
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(3), "{result}");
    for error in result["errors"].as_array().unwrap() {
        assert!(error["error"].as_str().is_some_and(|text| text.contains("no active document")), "{result}");
    }
    assert!(s.documents().is_empty(), "the inner run closed all three tabs");
    assert_eq!(std::fs::read_dir(&inner).unwrap().count(), 3, "the inner run wrote its three copies");
}
