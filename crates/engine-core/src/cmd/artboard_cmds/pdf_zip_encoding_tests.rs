//! Artboards to PDF's Encoding (#13651): `"zip"` writes every page as a lossless Flate image,
//! `"jpeg"` (the default) as before. The measured document: 800 x 400, bottom to top `Red Board`
//! 0,0-200,150 (white, pixel layer `Red` 220,30,30 at 20,20-120,100), `Blue Board`
//! 120,30-320,230 (white, pixel layer `Blue`) and `Empty Board` 600,0-700,100 (white, no layers).

use std::io::Read;

use super::pdf_overlapping_areas_tests::paint;
use super::*;
use photocraft_geom::Rect;

const CMD: &str = "file.export.artboardsToPdf";

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 800, "height": 400, "depth": 8, "resolution": 72, "background": "transparent"})).unwrap();
    let mut board = |rect: [i64; 4], name: &str| {
        s.execute("layer.new.artboard", json!({"rect": rect, "name": name, "background": "white"})).unwrap()["layer"].as_u64().unwrap()
    };
    let red = board([0, 0, 200, 150], "Red Board");
    let blue = board([120, 30, 200, 200], "Blue Board");
    board([600, 0, 100, 100], "Empty Board");
    paint(&mut s, red, "Red", Rect::new(20, 20, 120, 100), [220, 30, 30]);
    paint(&mut s, blue, "Blue", Rect::new(130, 40, 230, 140), [30, 30, 220]);
    s
}

/// Runs the command with `params` plus a fresh `path`; the PDF's bytes, or the command's error.
fn export(s: &mut Session, tag: &str, params: Value) -> Result<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("photocraft-pdf-zip-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = join(&dir.to_string_lossy(), "boards.pdf");
    let mut params = params;
    params["path"] = json!(path);
    let r = s.execute(CMD, params);
    let bytes = std::fs::read(&path);
    let _ = std::fs::remove_dir_all(&dir);
    r.map(|_| bytes.unwrap())
}

/// The first position of `needle` in `pdf` at or after `from` (byte offsets: streams are binary).
fn find(pdf: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    pdf[from..].windows(needle.len()).position(|w| w == needle).map(|i| from + i)
}

/// Every page image in page order: its dictionary and its stream bytes.
fn images(pdf: &[u8]) -> Vec<(String, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = find(pdf, at, b"/Subtype /Image") {
        let dict_end = find(pdf, start, b">>\nstream\n").unwrap();
        let dict = String::from_utf8(pdf[start..dict_end].to_vec()).unwrap();
        let len: usize = dict.split("/Length ").nth(1).unwrap().trim().parse().unwrap();
        let data = dict_end + b">>\nstream\n".len();
        out.push((dict, &pdf[data..data + len]));
        at = data + len;
    }
    out
}

fn inflate(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).unwrap();
    out
}

#[test]
fn zip_pages_are_flate_rgb_images_with_the_exact_pixels() {
    // Artboard Content Only: the default Include Overlapping Areas would show Blue Board's corner on Red Board's page.
    let pdf = export(&mut session(), "exact", json!({"encoding": "zip", "content": "artboard"})).unwrap();
    let pages = images(&pdf);
    assert_eq!(pages.len(), 3);
    for (dict, _) in &pages {
        assert!(dict.contains("/ColorSpace /DeviceRGB /BitsPerComponent 8") && dict.contains("/Filter /FlateDecode"), "{dict}");
    }
    assert!(find(&pdf, 0, b"/DCTDecode").is_none());
    // Page 1 is Red Board, 200 x 150: the red layer exactly 220,30,30, white elsewhere.
    assert!(pages[0].0.contains("/Width 200 /Height 150"), "{}", pages[0].0);
    let rgb = inflate(pages[0].1);
    assert_eq!(rgb.len(), 200 * 150 * 3);
    for (i, px) in rgb.as_chunks::<3>().0.iter().enumerate() {
        let (x, y) = (i % 200, i / 200);
        let want = if (20..120).contains(&x) && (20..100).contains(&y) { [220, 30, 30] } else { [255, 255, 255] };
        assert_eq!(*px, want, "page pixel {x},{y}");
    }
    // Empty Board is plain white.
    assert!(inflate(pages[2].1).iter().all(|&v| v == 255));
}

#[test]
fn zip_pdf_xref_offsets_point_at_their_objects() {
    let pdf = export(&mut session(), "xref", json!({"encoding": "zip"})).unwrap();
    assert!(images(&pdf).iter().all(|(dict, _)| dict.contains("/Filter /FlateDecode")));
    let xref = pdf.windows(6).rposition(|w| w == b"\nxref\n").unwrap() + 1;
    assert!(pdf.ends_with(format!("startxref\n{xref}\n%%EOF\n").as_bytes()));
    let table = String::from_utf8(pdf[xref..].to_vec()).unwrap();
    // Catalog, page tree, then page, contents and image per page.
    let offsets: Vec<usize> = table.lines().skip(3).take(2 + 3 * 3).map(|l| l[..10].parse().unwrap()).collect();
    assert_eq!(offsets.len(), 11);
    for (i, o) in offsets.iter().enumerate() {
        assert!(pdf[*o..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "object {}", i + 1);
    }
}

#[test]
fn zip_ignores_quality() {
    let mut s = session();
    let low = export(&mut s, "q0", json!({"encoding": "zip", "quality": 0})).unwrap();
    let high = export(&mut s, "q12", json!({"encoding": "zip", "quality": 12})).unwrap();
    assert_eq!(low, high);
}

#[test]
fn jpeg_and_no_encoding_give_todays_pdf() {
    let mut s = session();
    let default = export(&mut s, "none", json!({"quality": 8})).unwrap();
    assert_eq!(export(&mut s, "jpeg", json!({"encoding": "jpeg", "quality": 8})).unwrap(), default);
    assert_eq!(export(&mut s, "null", json!({"encoding": null, "quality": 8})).unwrap(), default);
    let pages = images(&default);
    assert_eq!(pages.len(), 3);
    assert!(pages.iter().all(|(dict, data)| dict.contains("/Filter /DCTDecode") && data.starts_with(&[0xff, 0xd8])));
}

#[test]
fn an_unknown_encoding_is_an_error() {
    let mut s = session();
    for (i, encoding) in [json!("lzw"), json!(5), json!(""), json!("ZIP"), json!(true)].into_iter().enumerate() {
        let err = export(&mut s, &format!("bad{i}"), json!({"encoding": encoding})).unwrap_err();
        assert!(err.to_string().contains("`encoding` must be"), "{encoding}: {err}");
    }
}

#[test]
fn the_params_document_the_encoding() {
    let spec = specs().into_iter().find(|c| c.id == CMD).unwrap();
    assert!(spec.params.contains(r#""encoding":"jpeg|zip"="jpeg""#), "{}", spec.params);
}
