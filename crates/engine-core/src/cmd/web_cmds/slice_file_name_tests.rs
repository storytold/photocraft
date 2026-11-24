//! #16662: a sliced Save for Web given a `path` names the generated slice files and the HTML
//! page after that file, not after the document; a slice with its own name keeps it.

use super::*;

/// "template.psd" with an unnamed slice over the top half and a slice named `footer` below it.
fn template() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 64, "height": 48, "name": "template.psd"})).unwrap();
    s.execute("slice.new", json!({"rect": [0, 0, 64, 24]})).unwrap();
    s.execute("slice.new", json!({"rect": [0, 24, 64, 24], "name": "footer"})).unwrap();
    s
}

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-web-slice-names-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// The file names (not paths) an export reported, and its HTML page's.
fn names(r: &Value) -> (Vec<String>, String) {
    let file = |v: &Value| std::path::Path::new(v.as_str().unwrap()).file_name().unwrap().to_string_lossy().into_owned();
    let mut files: Vec<String> = r["files"].as_array().unwrap().iter().map(file).collect();
    files.sort();
    (files, file(&r["html"]))
}

#[test]
fn a_path_names_the_generated_slice_files_and_the_page() {
    // As the dialog sends it (`dir` and the picked `path`), and as a script may (`path` alone).
    for with_dir in [true, false] {
        let dir = tmp(&format!("path-{with_dir}"));
        let mut s = template();
        let mut p = json!({"path": format!("{dir}/audi_en.png"), "slices": "all", "format": "png24", "html": true});
        if with_dir {
            p["dir"] = json!(dir);
        }
        let r = s.execute("file.export.saveForWebLegacy", p).unwrap();
        let (files, html) = names(&r);
        assert_eq!(files, ["audi_en_01.png", "footer.png"], "with dir {with_dir}: {r}");
        assert_eq!(html, "audi_en.html", "with dir {with_dir}");
        for written in r["files"].as_array().unwrap().iter().chain([&r["html"]]) {
            let written = written.as_str().unwrap();
            assert!(written.starts_with(&dir) && std::path::Path::new(written).is_file(), "with dir {with_dir}: {written}");
        }
        let page = std::fs::read_to_string(r["html"].as_str().unwrap()).unwrap();
        assert!(page.contains("images/audi_en_01.png") && !page.contains("template"), "with dir {with_dir}: {page}");
    }
}

#[test]
fn without_a_path_the_slices_keep_the_document_name() {
    let dir = tmp("dir");
    let r = template().execute("file.export.saveForWebLegacy", json!({"dir": dir, "format": "png24", "html": true})).unwrap();
    let (files, html) = names(&r);
    assert_eq!(files, ["footer.png", "template_01.png"], "{r}");
    assert_eq!(html, "template.html");
}
