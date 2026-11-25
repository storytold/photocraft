//! Package format selection follows the active document's saved format unless the caller supplies
//! an explicit package format. PSD/PSB inference is intentionally a documented `.pcraft` fallback
//! until the PSD writer can preserve external linked resources.

use super::*;

fn temp_dir(tag: &str) -> String {
    let path = std::env::temp_dir().join(format!("photocraft-package-format-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path.to_string_lossy().replace('\\', "/")
}

fn session_saved_as(path: &str) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "#336699"})).unwrap();
    s.saved_to(path.to_owned());
    s
}

fn package(s: &mut Session, dir: &str, extra: Value) -> Value {
    let mut params = json!({"dir": dir});
    params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    s.execute("file.package", params).unwrap()
}

#[test]
fn package_uses_the_saved_pcraft_format_by_default() {
    let dir = temp_dir("pcraft");
    let mut s = session_saved_as(&format!("{dir}/Artwork.pcraft"));
    let result = package(&mut s, &format!("{dir}/out"), json!({}));
    assert!(result["document"].as_str().unwrap().ends_with("Artwork/Artwork.pcraft"));
    assert!(result["warnings"].as_array().unwrap().is_empty());
    assert!(std::path::Path::new(result["document"].as_str().unwrap()).is_file());
}

#[test]
fn package_preserves_a_saved_tiff_extension_by_default() {
    let dir = temp_dir("tiff");
    let mut s = session_saved_as(&format!("{dir}/Artwork.tif"));
    let result = package(&mut s, &format!("{dir}/out"), json!({}));
    assert!(result["document"].as_str().unwrap().ends_with("Artwork/Artwork.tif"));
    assert!(std::path::Path::new(result["document"].as_str().unwrap()).is_file());
}

#[test]
fn inferred_psd_package_falls_back_to_pcraft_with_a_warning() {
    let dir = temp_dir("psd");
    let mut s = session_saved_as(&format!("{dir}/Artwork.psd"));
    let result = package(&mut s, &format!("{dir}/out"), json!({}));
    assert!(result["document"].as_str().unwrap().ends_with("Artwork/Artwork.pcraft"));
    let warnings = result["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|warning| warning.as_str().unwrap().contains("cannot preserve external links")), "{warnings:?}");
}

#[test]
fn explicit_package_format_overrides_the_saved_format() {
    let dir = temp_dir("override");
    let mut s = session_saved_as(&format!("{dir}/Artwork.pcraft"));
    let result = package(&mut s, &format!("{dir}/out"), json!({"format": "psd"}));
    assert!(result["document"].as_str().unwrap().ends_with("Artwork/Artwork.psd"));
    assert!(std::path::Path::new(result["document"].as_str().unwrap()).is_file());
    assert!(result["warnings"].as_array().unwrap().iter().all(|warning| !warning.as_str().unwrap().contains("cannot preserve external links")));
}
