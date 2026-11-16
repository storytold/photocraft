//! #13785: Export Preferences › PNG › Smaller File (8-bit), off by default, makes Quick Export
//! write a palette PNG with binary transparency: clear stays clear, a 50 % opaque pixel becomes
//! fully opaque in its own colour.

use super::*;
use photocraft_doc::{Layer, Size};
use photocraft_geom::Rect;

/// 20 x 10 with no Background: transparent except a 50 % opaque green (0,160,0) pixel at (5,5)
/// and an opaque blue one at (15,5).
fn session() -> Session {
    let mut s = Session::new();
    let mut doc = photocraft_doc::Document::new("quick", Size::new(20, 10), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let fmt = doc.pixel_format();
    let mut l = Layer::raster("Pixels", fmt);
    let surface = l.surface_mut().unwrap();
    surface.fill_rect(Rect::new(5, 5, 6, 6), &photocraft_raster::from_rgba(&fmt, [0.0, 160.0 / 255.0, 0.0, 128.0 / 255.0]));
    surface.fill_rect(Rect::new(15, 5, 16, 6), &photocraft_raster::from_rgba(&fmt, [0.0, 0.0, 1.0, 1.0]));
    doc.layers.push(l);
    s.add_document(doc, None);
    s.execute("file.export.exportPreferences", json!({"quickExportFormat": "png", "metadata": "none"})).unwrap();
    s
}

fn quick_export(s: &mut Session, name: &str) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("pc-quick-png8-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    s.execute("file.export.quickExport", json!({"path": path.to_string_lossy()})).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    bytes
}

fn pixel(png: &[u8], x: usize, y: usize) -> [u8; 4] {
    let doc = photocraft_io::import("x.png", png).unwrap().document;
    let w = doc.size.width as usize;
    photocraft_compose::flatten(&doc).into_px()[y * w + x].map(|v| (v * 255.0).round() as u8)
}

#[test]
fn it_is_off_by_default_and_a_file_without_it_loads_off() {
    assert!(!crate::prefs::Export::default().png_smaller_file);
    let export: crate::prefs::Export = serde_json::from_value(json!({"quickExportFormat": "png", "jpegQuality": 80})).unwrap();
    assert!(!export.png_smaller_file, "an older preferences file");
    let mut s = session();
    let r = s.execute("file.export.exportPreferences", json!({"pngSmallerFile": true})).unwrap();
    assert_eq!(r["values"]["pngSmallerFile"], true);
    assert!(s.prefs().export.png_smaller_file);
    assert!(s.execute("file.export.exportPreferences", json!({"pngSmallerFile": "yes"})).is_err(), "a boolean");
}

#[test]
fn on_quick_export_writes_a_palette_png_with_binary_transparency() {
    let mut s = session();
    s.execute("file.export.exportPreferences", json!({"pngSmallerFile": true})).unwrap();
    let png = quick_export(&mut s, "small.png");
    assert_eq!(&png[12..16], b"IHDR");
    assert_eq!(png[25], 3, "colour type 3: palette");
    assert!(png.windows(4).any(|w| w == b"tRNS"), "with tRNS");
    assert_eq!(pixel(&png, 0, 0)[3], 0, "transparent stays transparent");
    assert_eq!(pixel(&png, 5, 5), [0, 160, 0, 255], "50 % green becomes opaque in its own colour");
    assert_eq!(pixel(&png, 15, 5), [0, 0, 255, 255]);
}

#[test]
fn off_quick_export_writes_a_truecolour_png_as_before() {
    let mut s = session();
    let png = quick_export(&mut s, "full.png");
    assert_eq!(png[25], 6, "colour type 6: RGBA");
    assert_eq!(pixel(&png, 5, 5)[3], 128, "partial alpha kept");
}
