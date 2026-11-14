//! Artboards to Files writes each of the File Type list's types (#13635): PNG-8 an indexed
//! PNG, PNG-24 a truecolour one, Targa a `.tga`, PDF one multi-page file with the bottom
//! artboard first and each page the board's pixel size at 72 ppi.
use super::*;

/// The issue's test document: 800 x 400, transparent, Red Board 200 x 150 at the bottom of the
/// Layers panel, then Blue Board 200 x 200, then an empty transparent board on top. 300 ppi, so
/// a page sized from the document resolution would not be 72 ppi.
fn boards() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 400, "name": "abfiles-test", "background": "transparent"})).unwrap();
    for (name, rect, bg) in [
        ("Red Board", [0, 0, 200, 150], json!({"background": "custom", "color": "#ff0000"})),
        ("Blue Board", [300, 0, 200, 200], json!({"background": "custom", "color": "#0000ff"})),
        ("Empty Board", [600, 0, 100, 100], json!({"background": "transparent"})),
    ] {
        let mut p = json!({"rect": rect, "name": name});
        p.as_object_mut().unwrap().extend(bg.as_object().unwrap().clone());
        s.execute("layer.new.artboard", p).unwrap();
    }
    s.edit("ppi", |doc, _| {
        doc.resolution_dpi = 300.0;
        Ok(())
    })
    .unwrap();
    s
}

fn out_dir(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("photocraft-13635-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn export(s: &mut Session, dir: &str, format: &str) -> Vec<String> {
    let r = s.execute("file.export.artboardsToFiles", json!({"dir": dir, "format": format})).unwrap();
    r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

/// The PNG IHDR colour type: 2 truecolour, 3 indexed, 6 truecolour with alpha.
fn png_color_type(path: &str) -> u8 {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{path} is not a PNG");
    assert_eq!(&bytes[12..16], b"IHDR");
    bytes[25]
}

fn size_of(path: &str, ext: &str) -> (u32, u32) {
    let d = photocraft_io::import(&format!("x.{ext}"), &std::fs::read(path).unwrap()).unwrap().document;
    (d.size.width, d.size.height)
}

#[test]
fn png8_writes_indexed_pngs() {
    let mut s = boards();
    let dir = out_dir("png8");
    let files = export(&mut s, &dir, "png8");
    assert_eq!(files.len(), 3);
    for f in &files {
        assert!(f.ends_with(".png"), "{f}");
        assert_eq!(png_color_type(f), 3, "{f} is not indexed");
    }
    assert!(files[2].ends_with("abfiles-test_0002_Red Board.png"), "{files:?}");
    assert_eq!(size_of(&files[2], "png"), (200, 150));
    // Red stays red through the palette.
    let red = photocraft_io::import("x.png", &std::fs::read(&files[2]).unwrap()).unwrap().document;
    let px = photocraft_compose::flatten(&red).px.clone();
    assert!(px[0][0] > 0.99 && px[0][1] < 0.01 && px[0][2] < 0.01, "{:?}", px[0]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn png24_writes_truecolour_pngs_with_alpha_where_transparent() {
    let mut s = boards();
    let dir = out_dir("png24");
    let files = export(&mut s, &dir, "png24");
    assert_eq!(files.len(), 3);
    for f in &files {
        assert!(f.ends_with(".png"), "{f}");
        assert!(matches!(png_color_type(f), 2 | 6), "{f} is not truecolour");
    }
    // The empty board is transparent: the file keeps its alpha.
    assert!(files[0].ends_with("abfiles-test_0000_Empty Board.png"), "{files:?}");
    assert_eq!(png_color_type(&files[0]), 6);
    assert_eq!(size_of(&files[1], "png"), (200, 200));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Grayscale document's boards are still truecolour as PNG-24 (and keep their alpha), while
/// plain `png` keeps writing Grayscale as before.
#[test]
fn png24_is_truecolour_for_a_grayscale_document() {
    let mut s = boards();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    let dir = out_dir("png24-gray");
    let files = export(&mut s, &dir, "png24");
    assert_eq!(files.len(), 3);
    for f in &files {
        assert!(matches!(png_color_type(f), 2 | 6), "{f} is not truecolour");
    }
    assert_eq!(png_color_type(&files[0]), 6, "the transparent board keeps its alpha");
    assert_eq!(size_of(&files[2], "png"), (200, 150));
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Grayscale, "the document itself is unchanged");
    let files = export(&mut s, &dir, "png");
    assert!(files.iter().all(|f| matches!(png_color_type(f), 0 | 4)), "plain png stays Grayscale");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn targa_writes_tga_files() {
    let mut s = boards();
    let dir = out_dir("tga");
    for format in ["tga", "targa"] {
        let files = export(&mut s, &dir, format);
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|f| f.ends_with(".tga")), "{files:?}");
        assert_eq!(size_of(&files[2], "tga"), (200, 150));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pdf_writes_one_multi_page_file_bottom_artboard_first_at_72_ppi() {
    let mut s = boards();
    let dir = out_dir("pdf");
    let files = export(&mut s, &dir, "pdf");
    assert_eq!(files, vec![join(&dir, "abfiles-test.pdf")]);
    let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert_eq!(entries.len(), 1, "one file, not one per board");
    let bytes = std::fs::read(&files[0]).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(text.matches("/Type /Page ").count(), 3);
    assert_eq!(text.matches("/DCTDecode").count(), 3);
    // Page order: Red Board (bottom of the Layers panel), Blue Board, the empty board.
    let boxes: Vec<&str> = text.match_indices("/MediaBox [").map(|(i, _)| &text[i + 11..i + 11 + text[i + 11..].find(']').unwrap()]).collect();
    assert_eq!(boxes, vec!["0 0 200.000 150.000", "0 0 200.000 200.000", "0 0 100.000 100.000"]);
    // A prefix names the file.
    let r = s.execute("file.export.artboardsToFiles", json!({"dir": dir, "format": "pdf", "prefix": "pre_"})).unwrap();
    assert_eq!(r["files"], json!([join(&dir, "pre_.pdf")]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scripts_still_default_to_png() {
    let mut s = boards();
    let dir = out_dir("default");
    let r = s.execute("file.export.artboardsToFiles", json!({"dir": dir})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|f| f.ends_with(".png")), "{files:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_format_is_an_error_and_writes_nothing() {
    let mut s = boards();
    let dir = out_dir("unknown");
    for format in ["xyz", "png9", ""] {
        assert!(s.execute("file.export.artboardsToFiles", json!({"dir": dir, "format": format})).is_err(), "{format}");
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
