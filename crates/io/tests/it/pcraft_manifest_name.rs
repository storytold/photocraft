//! A `.pcraft` bundle's manifest reports the file it is in (#3312): writing a document to a new
//! name must not leave the previous one inside, on every path that produces `.pcraft` bytes.

use crate::common;

use common::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_format::zip::ZipReader;
use photocraft_io::*;

fn doc() -> photocraft_doc::Document {
    let mut d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
    d.name = "FirstTest.psd".into();
    d
}

/// `document.name` as the bundle actually stores it.
fn manifest_name(bytes: &[u8]) -> String {
    let archive = ZipReader::new(bytes).expect("zip");
    let manifest: serde_json::Value = serde_json::from_slice(&archive.read_by_name("manifest.json", usize::MAX).expect("manifest")).expect("manifest json");
    manifest["document"]["name"].as_str().expect("document.name").to_string()
}

/// Save As to a new name: the written file names itself, so reopening it agrees with the session
/// that wrote it instead of reporting the file it came from.
#[test]
fn export_names_the_manifest_after_the_file() {
    let r = export(&doc(), "/tmp/somewhere/_nametest.pcraft", &ExportOptions::default()).expect("export");
    assert_eq!(manifest_name(&r.bytes), "_nametest.pcraft");
    assert_eq!(photocraft_format::load_from_bytes(&r.bytes).expect("load").name, "_nametest.pcraft");
}

/// A bare extension names no file and leaves the document's own name alone.
#[test]
fn a_bare_extension_names_no_file() {
    let r = export(&doc(), "pcraft", &ExportOptions::default()).expect("export");
    assert_eq!(manifest_name(&r.bytes), "FirstTest.psd");
}

/// A bundle copied or renamed outside the app opens under its new file name, as every other
/// format does, not under the name it was saved with (#13412).
#[test]
fn a_renamed_bundle_opens_under_its_file_name() {
    let saved = export(&doc(), "X.pcraft", &ExportOptions::default()).expect("export").bytes;
    assert_eq!(import("Y.pcraft", &saved).expect("import").document.name, "Y.pcraft");
    assert_eq!(import_for_open("/copies/Y.pcraft", &saved, &ImportOptions::default()).expect("open").0.document.name, "Y.pcraft");
}

/// Opened from disk, the name is the file's on disk.
#[test]
fn a_renamed_bundle_on_disk_opens_under_its_file_name() {
    let saved = export(&doc(), "X.pcraft", &ExportOptions::default()).expect("export").bytes;
    let dir = std::env::temp_dir().join(format!("photocraft-renamed-bundle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("Y.pcraft");
    std::fs::write(&path, &saved).expect("write");
    let opened = import_file("Y.pcraft", &path, &ImportOptions::default(), &photocraft_raster::Interrupt::NONE);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(opened.expect("open").document.name, "Y.pcraft");
}
