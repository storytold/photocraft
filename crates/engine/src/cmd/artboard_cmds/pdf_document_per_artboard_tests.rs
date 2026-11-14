//! Artboards to PDF's Document Per Artboard (#13649): one single-page PDF per board, named
//! `<prefix><artboard name>.pdf` as the reference application does, next to the default
//! Multi-Page Document (one file, a page per board).

use super::*;

/// The issue's measured boards, bottom to top in the Layers panel: name and `[x, y, w, h]`.
const MEASURED: [(&str, [u32; 4]); 3] = [("Red Board", [0, 0, 200, 150]), ("Blue Board", [120, 30, 200, 200]), ("Empty Board", [600, 0, 100, 100])];

/// The issue's test document: 800 x 400, RGB, 8 bit, 72 ppi, named `abpdf-test`, with `boards`
/// added bottom to top.
fn session(boards: &[(&str, [u32; 4])]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "abpdf-test", "width": 800, "height": 400, "depth": 8, "resolution": 72})).unwrap();
    for (name, rect) in boards {
        s.execute("layer.new.artboard", json!({"rect": rect, "name": name})).unwrap();
    }
    s
}

/// A fresh empty folder for one test.
fn folder(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-13649-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The file names in `dir`, sorted.
fn listing(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// The pages of the PDF at `path`: their MediaBox sizes in points.
fn pages(path: &std::path::Path) -> Vec<String> {
    let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
    text.split("/MediaBox [0 0 ").skip(1).map(|rest| rest.split(']').next().unwrap().to_string()).collect()
}

fn sorted(mut v: Vec<&str>) -> Vec<String> {
    v.sort_unstable();
    v.into_iter().map(str::to_string).collect()
}

#[test]
fn document_per_artboard_writes_one_single_page_file_per_board() {
    let mut s = session(&MEASURED);
    let dir = folder("per-board");
    let d = dir.to_string_lossy().into_owned();
    let r = s.execute("file.export.artboardsToPdf", json!({"mode": "perArtboard", "dir": d, "prefix": "pre_"})).unwrap();
    assert_eq!(listing(&dir), sorted(vec!["pre_Blue Board.pdf", "pre_Empty Board.pdf", "pre_Red Board.pdf"]));
    for (name, size) in [("pre_Red Board.pdf", "200.000 150.000"), ("pre_Blue Board.pdf", "200.000 200.000"), ("pre_Empty Board.pdf", "100.000 100.000")] {
        assert_eq!(pages(&dir.join(name)), [size], "{name}");
    }
    // The result lists every written file.
    let mut files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect();
    files.sort();
    let mut want: Vec<String> = listing(&dir).iter().map(|n| join(&d, n)).collect();
    want.sort();
    assert_eq!(files, want);
    assert_eq!(r["pages"], 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn multi_page_document_stays_the_default() {
    for mode in [None, Some(json!("multiPage")), Some(Value::Null)] {
        let mut s = session(&MEASURED);
        let dir = folder("multi");
        let path = join(&dir.to_string_lossy(), "abpdf-test.pdf");
        let mut p = json!({"path": path});
        if let Some(mode) = &mode {
            p["mode"] = mode.clone();
        }
        let r = s.execute("file.export.artboardsToPdf", p).unwrap();
        assert_eq!(listing(&dir), ["abpdf-test.pdf"], "{mode:?}");
        assert_eq!(pages(&dir.join("abpdf-test.pdf")).len(), 3, "{mode:?}");
        assert_eq!(r["files"], json!([path]), "{mode:?}");
        assert_eq!(r["path"], json!(path), "{mode:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn folder_and_prefix_default_to_those_of_path() {
    // The dialog's Path is `<folder>/<document name>.pdf`: per artboard, the files go into that
    // folder as `<document name><artboard name>.pdf`.
    let mut s = session(&MEASURED);
    let dir = folder("from-path");
    let path = join(&dir.to_string_lossy(), "abpdf-test.pdf");
    s.execute("file.export.artboardsToPdf", json!({"mode": "perArtboard", "path": path})).unwrap();
    assert_eq!(listing(&dir), sorted(vec!["abpdf-testBlue Board.pdf", "abpdf-testEmpty Board.pdf", "abpdf-testRed Board.pdf"]));
    let _ = std::fs::remove_dir_all(&dir);
    // With only a folder, the prefix is the document name; an empty prefix adds nothing.
    let dir = folder("doc-name");
    let d = dir.to_string_lossy().into_owned();
    let red = s.active().unwrap().doc.artboards().into_iter().find(|b| b.1 == "Red Board").unwrap().0.0;
    s.execute("file.export.artboardsToPdf", json!({"mode": "perArtboard", "dir": d, "artboards": [red]})).unwrap();
    s.execute("file.export.artboardsToPdf", json!({"mode": "perArtboard", "dir": d, "prefix": "", "artboards": [red]})).unwrap();
    assert_eq!(listing(&dir), ["Red Board.pdf", "abpdf-testRed Board.pdf"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn characters_a_file_name_cannot_hold_become_underscores() {
    let mut s = session(&[("a/b:c*d", [0, 0, 20, 20]), ("con", [40, 0, 20, 20])]);
    let dir = folder("sanitize");
    let d = dir.to_string_lossy().into_owned();
    s.execute("file.export.artboardsToPdf", json!({"mode": "perArtboard", "dir": d, "prefix": "p?"})).unwrap();
    assert_eq!(listing(&dir), ["p_a_b_c_d.pdf", "p_con.pdf"]);
    let _ = std::fs::remove_dir_all(&dir);
    // A Windows device name stays usable on its own.
    assert_eq!(pdf_file_stems("", &["con".into()]), ["con_"]);
}

#[test]
fn boards_with_equal_names_still_get_a_file_each() {
    // In export order: the first of a name keeps it, a later one gets a free suffix, also when
    // the names differ only in letter case or after sanitizing, and never another board's name.
    let names: Vec<String> = ["A", "a", "A-2", "A/", "A:"].into_iter().map(String::from).collect();
    assert_eq!(pdf_file_stems("p", &names), ["pA", "pa-3", "pA-2", "pA_", "pA_-2"]);
    let mut s = session(&[("Board", [0, 0, 20, 20]), ("board", [40, 0, 20, 20]), ("Board", [80, 0, 20, 20])]);
    let dir = folder("collide");
    let d = dir.to_string_lossy().into_owned();
    let r = s.execute("file.export.artboardsToPdf", json!({"mode": "perArtboard", "dir": d, "prefix": ""})).unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    let keys: HashSet<String> = files.iter().map(|f| f.as_str().unwrap().to_lowercase()).collect();
    assert_eq!(keys.len(), 3, "{files:?}");
    for f in files {
        assert_eq!(pages(std::path::Path::new(f.as_str().unwrap())).len(), 1, "{f}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_board_that_cannot_be_exported_writes_nothing() {
    let mut s = session(&MEASURED);
    s.execute("layer.new.artboard", json!({"x": i32::MIN, "width": 50, "height": 50, "name": "Far"})).unwrap();
    let dir = std::env::temp_dir().join(format!("photocraft-13649-far-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.to_string_lossy().into_owned();
    assert!(s.execute("file.export.artboardsToPdf", json!({"mode": "perArtboard", "dir": d, "prefix": "pre_"})).is_err());
    assert!(!dir.exists(), "nothing was written");
}

#[test]
fn a_wrong_mode_or_missing_destination_is_an_error_not_a_panic() {
    let mut s = session(&MEASURED);
    let dir = std::env::temp_dir().join(format!("photocraft-13649-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = join(&dir.to_string_lossy(), "x.pdf");
    for mode in [json!("nope"), json!("PerArtboard"), json!(""), json!(1), json!(true), json!(["perArtboard"]), json!({})] {
        let err = s.execute("file.export.artboardsToPdf", json!({"mode": mode, "path": path})).unwrap_err();
        assert!(matches!(err, EngineError::BadParams { .. }), "{mode}: {err}");
    }
    // Per artboard needs a folder: from "dir" or from "path".
    for p in [json!({"mode": "perArtboard"}), json!({"mode": "perArtboard", "dir": "", "path": ""}), json!({"mode": "perArtboard", "path": 5})] {
        let err = s.execute("file.export.artboardsToPdf", p.clone()).unwrap_err();
        assert!(matches!(err, EngineError::BadParams { .. }), "{p}: {err}");
    }
    assert!(!dir.exists(), "nothing was written");
}

#[test]
fn the_params_doc_names_the_mode() {
    let spec = specs().into_iter().find(|c| c.id == "file.export.artboardsToPdf").unwrap();
    for word in ["\"mode\"", "multiPage", "perArtboard", "\"dir\"", "\"prefix\"", "files"] {
        assert!(spec.params.contains(word), "{word}");
    }
}
