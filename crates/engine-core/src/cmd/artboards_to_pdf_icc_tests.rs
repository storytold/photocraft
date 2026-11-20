//! Artboards to PDF tags its pages with the document's colour profile (#13653): ticked, each page's
//! resources map its DeviceRGB image to the profile through a `DefaultRGB` ICCBased colour space;
//! unticked, or with a profile that can't describe RGB, no page has a ColorSpace resource.

use super::*;
use std::sync::Arc;

use craft_cms::Builtin;

/// An RGB document with two artboards, tagged with `profile`.
fn session(profile: Option<Arc<Vec<u8>>>) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 200, "depth": 8, "resolution": 72})).unwrap();
    s.execute("layer.new.artboard", json!({"rect": [0, 0, 100, 100], "name": "One"})).unwrap();
    s.execute("layer.new.artboard", json!({"rect": [200, 0, 100, 50], "name": "Two"})).unwrap();
    Arc::make_mut(s.active_mut().unwrap().doc_mut()).icc_profile = profile;
    s
}

/// Runs Artboards to PDF with `params` and returns the file's bytes.
fn export(s: &mut Session, tag: &str, params: Value) -> Result<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("photocraft-pdf-icc-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = join(&dir.to_string_lossy(), "boards.pdf");
    let mut params = params;
    params["path"] = json!(path);
    let r = s.execute("file.export.artboardsToPdf", params).map(|_| std::fs::read(&path).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
    r
}

/// Each page object's dictionary, in file order.
fn pages(pdf: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(pdf).into_owned();
    text.split(" 0 obj\n").filter(|o| o.starts_with("<< /Type /Page ")).map(|o| o.lines().next().unwrap_or_default().to_string()).collect()
}

/// The body of object `n` (from after its `n 0 obj` line), found through the xref table.
fn object(pdf: &[u8], n: usize) -> &[u8] {
    let text = String::from_utf8_lossy(pdf).into_owned();
    let xref = text.rfind("\nxref\n").unwrap() + 1;
    let line = text[xref..].lines().nth(2 + n).unwrap();
    let at: usize = line[..10].parse().unwrap();
    let head = format!("{n} 0 obj\n");
    assert!(pdf[at..].starts_with(head.as_bytes()), "xref entry {n} points at its object");
    &pdf[at + head.len()..]
}

/// The profile bytes and dictionary of the ICCBased stream each page's DefaultRGB names.
fn default_rgb_profiles(pdf: &[u8]) -> Vec<(String, Vec<u8>)> {
    pages(pdf)
        .iter()
        .map(|page| {
            let rest = page.split("/DefaultRGB [/ICCBased ").nth(1).unwrap_or_else(|| panic!("no DefaultRGB in {page}"));
            let n: usize = rest.split(' ').next().unwrap().parse().unwrap();
            let body = object(pdf, n);
            let start = body.windows(8).position(|w| w == b"\nstream\n").unwrap();
            let dict = String::from_utf8_lossy(&body[..start]).into_owned();
            let len: usize = dict.split("/Length ").nth(1).unwrap().split([' ', '>']).next().unwrap().parse().unwrap();
            let data = body[start + 8..start + 8 + len].to_vec();
            assert!(body[start + 8 + len..].starts_with(b"\nendstream\nendobj\n"));
            (dict, data)
        })
        .collect()
}

#[test]
fn ticked_by_default_every_page_maps_rgb_to_the_documents_profile() {
    let profile = Builtin::DisplayP3.profile().to_bytes();
    let mut s = session(Some(profile.clone()));
    let pdf = export(&mut s, "default", json!({})).unwrap();
    let found = default_rgb_profiles(&pdf);
    assert_eq!(found.len(), 2, "one per page");
    for (dict, data) in &found {
        assert_eq!(data.as_slice(), profile.as_slice(), "the PDF holds the document's profile byte for byte");
        assert!(dict.contains("/N 3"), "{dict}");
    }
    // The page images stay DeviceRGB; DefaultRGB is what tags them.
    assert_eq!(String::from_utf8_lossy(&pdf).matches("/ColorSpace /DeviceRGB").count(), 2);
}

#[test]
fn ticked_explicitly_writes_the_profile_once_for_all_pages() {
    let profile = Builtin::Srgb.profile().to_bytes();
    let mut s = session(Some(profile.clone()));
    let pdf = export(&mut s, "true", json!({"includeIcc": true})).unwrap();
    assert_eq!(String::from_utf8_lossy(&pdf).matches("/ICCBased").count(), 2);
    assert_eq!(String::from_utf8_lossy(&pdf).matches("/N 3").count(), 1, "a single profile object");
    assert!(default_rgb_profiles(&pdf).iter().all(|(_, data)| data.as_slice() == profile.as_slice()));
}

#[test]
fn unticked_no_page_has_a_colour_space_resource() {
    let mut s = session(Some(Builtin::Srgb.profile().to_bytes()));
    let pdf = export(&mut s, "false", json!({"includeIcc": false})).unwrap();
    let pages = pages(&pdf);
    assert_eq!(pages.len(), 2);
    assert!(pages.iter().all(|p| !p.contains("/ColorSpace")), "{pages:?}");
    assert!(!String::from_utf8_lossy(&pdf).contains("ICCBased"));
}

#[test]
fn a_profile_that_is_not_rgb_or_unreadable_is_left_out() {
    let gray = Builtin::SGray.profile().to_bytes();
    let cmyk = Builtin::CoatedCmyk.profile().to_bytes();
    for (tag, profile) in [("gray", Some(gray)), ("cmyk", Some(cmyk)), ("short", Some(Arc::new(b"RGB junk".to_vec()))), ("none", None)] {
        let mut s = session(profile);
        let pdf = export(&mut s, tag, json!({"includeIcc": true})).unwrap_or_else(|e| panic!("{tag}: {e}"));
        let pages = pages(&pdf);
        assert_eq!(pages.len(), 2, "{tag}");
        assert!(pages.iter().all(|p| !p.contains("/ColorSpace")), "{tag}: {pages:?}");
    }
}

#[test]
fn a_wrong_typed_include_icc_is_an_error() {
    let mut s = session(Some(Builtin::Srgb.profile().to_bytes()));
    for v in [json!("yes"), json!(1), json!([true])] {
        let e = export(&mut s, "bad", json!({"includeIcc": v})).expect_err(&format!("includeIcc {v}"));
        assert!(e.to_string().contains("includeIcc"), "{e}");
    }
}

/// The profile comes after every page's objects, a soft mask included (Include Background in
/// Export unticked), and every page points at it.
#[test]
fn xref_offsets_point_at_every_object_with_the_profile() {
    let jpeg = vec![0xff, 0xd8, 0xff, 0xd9];
    let pdf = raster_pdf_masked(&[(2, 3, 72.0, jpeg.clone(), None), (4, 5, 72.0, jpeg, Some(vec![0x78, 0x9c]))], Some(b"profile"));
    let text = String::from_utf8_lossy(&pdf).into_owned();
    assert!(text.contains("/Size 11 "), "catalog, page tree, 3 objects for the first page, 4 with its mask for the second, and the profile");
    for n in 1..=10 {
        object(&pdf, n);
    }
    assert!(object(&pdf, 10).starts_with(b"<< /N 3 "));
    assert_eq!(text.matches("/DefaultRGB [/ICCBased 10 0 R]").count(), 2);
}
