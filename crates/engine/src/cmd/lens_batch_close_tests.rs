//! File › Automate › Lens Correction closes a listed source that is open in the app (#13991).

use super::*;

type Res<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A fresh folder holding `a.png`, `b.png` and `c.png` (40 x 30, flat), and the output folder.
fn fixture(name: &str) -> Res<(std::path::PathBuf, [String; 3], String)> {
    let dir = std::env::temp_dir().join(format!("pc-lens-close-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "#808080"}))?;
    let files = ["a.png", "b.png", "c.png"].map(|f| dir.join(f).to_string_lossy().into_owned());
    let doc = &s.active().ok_or("no document")?.doc;
    for path in &files {
        crate::file_cmds::save_doc(doc, path, None)?;
    }
    let output = dir.join("out").to_string_lossy().into_owned();
    Ok((dir, files, output))
}

fn open(s: &mut Session, path: &str) -> Res<()> {
    s.execute("file.openAs", json!({"path": path}))?;
    Ok(())
}

/// An Untitled document with a second layer, the document the run must leave alone.
fn untitled(s: &mut Session) -> Res<usize> {
    s.execute("file.new", json!({"width": 20, "height": 20}))?;
    s.execute("layer.new.layer", json!({"name": "kept"}))?;
    Ok(s.active_index().ok_or("no active document")?)
}

fn run(s: &mut Session, input: Value, output: &str) -> Res<Value> {
    let r = s.execute(LENS_BATCH, json!({"input": input, "output": output}))?;
    assert!(r["errors"].as_array().is_some_and(Vec::is_empty), "{r}");
    Ok(r)
}

fn paths(s: &Session) -> Vec<Option<String>> {
    s.documents().iter().map(|d| d.path.clone()).collect()
}

#[test]
fn an_open_unmodified_source_is_closed_and_the_other_documents_stay() {
    let (dir, [a, b, _], out) = fixture("open").unwrap();
    let mut s = Session::new();
    open(&mut s, &a).unwrap();
    let kept = untitled(&mut s).unwrap();
    assert!(!s.documents()[0].is_dirty(), "the opened source is saved and unmodified");
    let steps = s.active().unwrap().history.entries().len();

    let r = run(&mut s, json!([a, b]), &out).unwrap();

    assert_eq!(r["files"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(paths(&s), [None], "only the Untitled document is left open");
    assert_eq!(s.active_index(), Some(kept - 1), "the Untitled document is still the active one");
    let d = s.active().unwrap();
    assert_eq!(d.doc.layers.len(), 2, "its layers are untouched");
    assert_eq!(d.history.entries().len(), steps, "its history is untouched");
    assert!(d.is_dirty(), "it keeps its unsaved state");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_open_source_that_is_the_active_document_is_closed() {
    let (dir, [a, b, _], out) = fixture("active").unwrap();
    let mut s = Session::new();
    untitled(&mut s).unwrap();
    open(&mut s, &b).unwrap();

    run(&mut s, json!([a, b]), &out).unwrap();

    assert_eq!(paths(&s), [None]);
    assert_eq!(s.active_index(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_open_sources_are_both_closed_and_the_active_untitled_document_stays_active() {
    let (dir, [a, b, _], out) = fixture("both").unwrap();
    let mut s = Session::new();
    open(&mut s, &a).unwrap();
    let kept = untitled(&mut s).unwrap();
    open(&mut s, &b).unwrap();
    s.set_active(kept);

    run(&mut s, json!([a, b]), &out).unwrap();

    assert_eq!(paths(&s), [None]);
    assert_eq!(s.active_index(), Some(0), "the Untitled document is still the active one");
    assert_eq!(s.active().unwrap().doc.layers.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_source_listed_under_another_spelling_of_its_path_is_closed() {
    let (dir, [a, b, _], out) = fixture("alias").unwrap();
    let mut s = Session::new();
    open(&mut s, &a).unwrap();
    // `..` (unlike `.`) is not dropped when paths are compared, so only the file on disk matches.
    std::fs::create_dir(dir.join("sub")).unwrap();
    let alias = dir.join("sub").join("..").join("a.png").to_string_lossy().into_owned();
    assert_ne!(std::path::Path::new(&alias), std::path::Path::new(&a));

    run(&mut s, json!([alias, b]), &out).unwrap();

    assert!(s.documents().is_empty(), "{:?}", paths(&s));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_open_saved_document_that_is_not_a_source_stays_open() {
    let (dir, [a, b, c], out) = fixture("other").unwrap();
    let mut s = Session::new();
    open(&mut s, &c).unwrap();

    run(&mut s, json!([a, b]), &out).unwrap();

    assert_eq!(paths(&s), [Some(c.clone())]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_open_source_with_unsaved_changes_stays_open() {
    let (dir, [a, b, _], out) = fixture("dirty").unwrap();
    let mut s = Session::new();
    open(&mut s, &a).unwrap();
    s.execute("layer.new.layer", json!({"name": "unsaved"})).unwrap();
    let revision = s.active().unwrap().revision;
    assert!(s.active().unwrap().is_dirty());

    run(&mut s, json!([a, b]), &out).unwrap();

    assert_eq!(paths(&s), [Some(a.clone())], "the modified source is not closed");
    let d = s.active().unwrap();
    assert!(d.is_dirty() && d.revision == revision, "its changes are kept, not discarded");
    assert_eq!(d.doc.layers.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Not measured: the document may be the only copy left once its file is gone.
#[test]
fn an_open_source_whose_correction_fails_stays_open() {
    let (dir, [a, b, _], out) = fixture("failed").unwrap();
    let mut s = Session::new();
    open(&mut s, &a).unwrap();
    std::fs::remove_file(&a).unwrap();

    let r = s.execute(LENS_BATCH, json!({"input": [a, b], "output": out})).unwrap();

    let errors = r["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{r}");
    assert_eq!(errors[0]["file"], json!(a), "{r}");
    assert_eq!(paths(&s), [Some(a.clone())], "the failed source is not closed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sources_that_are_not_open_leave_nothing_open() {
    let (dir, [a, b, _], out) = fixture("closed").unwrap();
    let mut s = Session::new();

    let r = run(&mut s, json!([a, b]), &out).unwrap();

    assert_eq!(r["files"].as_array().unwrap().len(), 2, "{r}");
    assert!(s.documents().is_empty());
    assert!(std::path::Path::new(&out).join("a.png").is_file() && std::path::Path::new(&out).join("b.png").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Not measured: Folder mode with a document from that folder open keeps today's behaviour.
#[test]
fn folder_mode_keeps_an_open_source_open() {
    let (dir, [a, _, _], out) = fixture("folder").unwrap();
    let mut s = Session::new();
    open(&mut s, &a).unwrap();

    run(&mut s, json!(dir.to_string_lossy()), &out).unwrap();

    assert_eq!(paths(&s), [Some(a.clone())]);
    let _ = std::fs::remove_dir_all(&dir);
}
