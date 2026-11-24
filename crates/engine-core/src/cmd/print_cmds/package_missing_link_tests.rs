//! Package stops with an error, before the destination is looked at and before anything is
//! written, when a linked file at any depth is missing, not followed or not placeable (#14076).

use std::path::Path;

use super::package_enabled_tests::{add_psd_embedded, embedded, greyed, linked, nested, png, saved_with, tmp};
use super::*;

/// Runs File › Package into `<dir>/pkg`.
fn package_into(s: &mut Session, dir: &str) -> Result<Value> {
    s.execute("file.package", json!({"dir": format!("{dir}/pkg")}))
}

/// Package failed with the reference application's message, Package was still enabled, and
/// nothing was written: no package folder, no `Links`, no document copy.
fn assert_refused(s: &mut Session, dir: &str) {
    assert_eq!(greyed(s), None, "the menu item stays enabled");
    let e = package_into(s, dir).unwrap_err();
    assert_eq!(e.to_string(), PACKAGE_MISSING_LINK);
    assert!(!Path::new(&format!("{dir}/pkg")).exists(), "something was written into {dir}/pkg");
}

/// The measured case: `missing.pcraft` with a layer `Gone` linked to a deleted `gone.png` was
/// written with `gone.png` listed in `missing` only.
#[test]
fn a_missing_linked_file_is_an_error_and_nothing_is_written() {
    let dir = tmp("missing-link-missing");
    let gone = png(&format!("{dir}/gone.png"), "#ff0000");
    let mut s = saved_with(&dir, vec![linked("Gone", &gone)]);
    std::fs::remove_file(&gone).unwrap();
    assert_refused(&mut s, &dir);
}

/// Links found before the missing one were already copied into `Links`: no longer.
#[test]
fn links_before_the_missing_one_are_not_copied() {
    let dir = tmp("missing-link-partial");
    let red = png(&format!("{dir}/red.png"), "#ff0000");
    let gone = png(&format!("{dir}/gone.png"), "#00ff00");
    let blue = png(&format!("{dir}/blue.png"), "#0000ff");
    let mut s = saved_with(&dir, vec![linked("Red", &red), linked("Gone", &gone), linked("Blue", &blue)]);
    std::fs::remove_file(&gone).unwrap();
    assert_refused(&mut s, &dir);
    // With the file back, the same document packages all three.
    png(&gone, "#00ff00");
    let r = package_into(&mut s, &dir).unwrap();
    assert_eq!(r["links"].as_array().unwrap().len(), 3, "{r}");
    assert!(r.get("missing").is_none(), "{r}");
}

/// The error comes before the destination: without `dir` it is still the missing link.
#[test]
fn the_missing_link_comes_before_the_destination() {
    let dir = tmp("missing-link-before-dir");
    let gone = png(&format!("{dir}/gone.png"), "#ff0000");
    let mut s = saved_with(&dir, vec![linked("Gone", &gone)]);
    std::fs::remove_file(&gone).unwrap();
    assert!(package_link_missing(&s));
    assert_eq!(s.execute("file.package", json!({})).unwrap_err().to_string(), PACKAGE_MISSING_LINK);
    // A document whose links are all there still asks for the destination.
    png(&gone, "#ff0000");
    assert!(!package_link_missing(&s));
    let e = s.execute("file.package", json!({})).unwrap_err().to_string();
    assert!(e.contains("missing \"dir\""), "{e}");
}

/// A missing link inside an embedded smart object, at any depth, stops Package too.
#[test]
fn a_missing_nested_link_is_an_error() {
    let dir = tmp("missing-link-nested");
    let red = png(&format!("{dir}/red.png"), "#ff0000");
    let gone = png(&format!("{dir}/gone.png"), "#00ff00");
    for levels in [1, 3] {
        let mut s = saved_with(&dir, vec![linked("Red", &red), nested(linked("Gone", &gone), levels)]);
        std::fs::remove_file(&gone).unwrap();
        assert_refused(&mut s, &dir);
        png(&gone, "#00ff00");
    }
}

/// A path that is not followed and a file PhotoCraft cannot place are handled like a missing
/// file, as Package already treated them.
#[test]
fn an_unfollowed_or_unplaceable_link_is_an_error() {
    let dir = tmp("missing-link-unreadable");
    let text = format!("{dir}/readme.png");
    std::fs::write(&text, b"plain text, not an image").unwrap();
    for path in [r"\\192.0.2.1\share\x.png", text.as_str()] {
        let mut s = saved_with(&dir, vec![linked("Link", path)]);
        assert_refused(&mut s, &dir);
    }
}

/// Smart objects a PSD keeps embedded are no links: Package neither stops on them nor copies
/// them, and the packaged copy keeps them as they are.
#[test]
fn smart_objects_embedded_in_a_psd_are_not_missing_links() {
    let dir = tmp("missing-link-psd-embedded");
    let red = png(&format!("{dir}/red.png"), "#ff0000");
    let blue = png(&format!("{dir}/blue.png"), "#0000ff");
    let mut s = saved_with(&dir, vec![linked("Red", &red)]);
    add_psd_embedded(&mut s, "Placed", &blue);
    assert!(!package_link_missing(&s));
    let r = package_into(&mut s, &dir).unwrap();
    assert_eq!(r["links"], json!([format!("{dir}/pkg/Pack/Links/red.png")]));
    let back = photocraft_io::import("Pack.pcraft", &std::fs::read(r["document"].as_str().unwrap()).unwrap()).unwrap().document;
    let placed = back.layers.iter().find(|l| l.name == "Placed").unwrap();
    assert!(matches!(&placed.content, LayerContent::Smart(so) if so.source == SmartSource::Linked { path: "Placed-uuid".into() }));
}

/// An embedded smart object without links inside is no missing link.
#[test]
fn embedded_contents_without_links_package() {
    let dir = tmp("missing-link-embedded");
    let red = png(&format!("{dir}/red.png"), "#ff0000");
    let mut s = saved_with(&dir, vec![linked("Red", &red), embedded("Outer", vec![])]);
    assert!(package_into(&mut s, &dir).is_ok());
}
