//! #13902: Image Processor's Include ICC Profile (`includeIcc`, on by default): off leaves the
//! profile out of JPEG and TIFF results; PSD and the source documents keep theirs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use serde_json::{Value, json};

use crate::Session;
use crate::error::EngineError;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "white", "profile": "srgb"})).unwrap();
    assert!(s.active().unwrap().doc.icc_profile.is_some(), "the source is tagged");
    s
}

fn run(s: &mut Session, extra: Value) -> tempfile::TempDir {
    let out = tempfile::Builder::new().prefix("photocraft-ip-icc-").tempdir().unwrap();
    let mut p = json!({"source": "current", "output": out.path().to_string_lossy()});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    let r = s.execute("file.scripts.imageProcessor", p).unwrap();
    assert!(r["errors"].as_array().is_none_or(Vec::is_empty), "{r}");
    out
}

/// The profile embedded in the one result under `out` whose extension starts with `ext` (`tif`
/// also finds `.tiff`), if any.
fn embedded(out: &std::path::Path, ext: &str) -> Option<Vec<u8>> {
    let path = walk(out).into_iter().find(|p| p.extension().is_some_and(|e| e.to_string_lossy().starts_with(ext))).unwrap_or_else(|| panic!("a .{ext} result"));
    let bytes = std::fs::read(&path).unwrap();
    photocraft_io::import(&path.to_string_lossy(), &bytes).unwrap().document.icc_profile.map(|p| p.to_vec())
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .flat_map(|e| {
            let p = e.unwrap().path();
            if p.is_dir() { walk(&p) } else { vec![p] }
        })
        .collect()
}

#[test]
fn off_leaves_the_profile_out_of_jpeg_and_tiff() {
    let mut s = session();
    for (format, ext) in [("jpg", "jpg"), ("tiff", "tif")] {
        let out = run(&mut s, json!({"format": format, "includeIcc": false}));
        assert_eq!(embedded(out.path(), ext), None, "{format}: no profile");
    }
    // The three-format run too; its PSD keeps the profile.
    let out = run(&mut s, json!({"jpeg": {}, "psd": {}, "tiff": {}, "includeIcc": false}));
    assert_eq!(embedded(out.path(), "jpg"), None);
    assert_eq!(embedded(out.path(), "tif"), None);
    assert!(embedded(out.path(), "psd").is_some(), "PSD keeps today's behaviour");
}

#[test]
fn on_or_omitted_embeds_it_as_today() {
    let mut s = session();
    for extra in [json!({"format": "jpg"}), json!({"format": "jpg", "includeIcc": true}), json!({"jpeg": {}, "tiff": {}})] {
        let out = run(&mut s, extra.clone());
        assert!(embedded(out.path(), "jpg").is_some(), "{extra}");
    }
}

#[test]
fn a_bad_value_is_an_error_and_the_source_keeps_its_profile() {
    let mut s = session();
    let before = s.active().unwrap().doc.icc_profile.clone();
    for bad in [json!("no"), json!(0), json!([false])] {
        let out = tempfile::tempdir().unwrap();
        let r =
            s.execute("file.scripts.imageProcessor", json!({"source": "current", "output": out.path().to_string_lossy(), "format": "jpg", "includeIcc": bad}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert!(walk(out.path()).is_empty(), "{bad}: nothing written");
    }
    run(&mut s, json!({"format": "jpg", "includeIcc": false}));
    assert_eq!(s.active().unwrap().doc.icc_profile, before, "the source keeps its profile");
}
