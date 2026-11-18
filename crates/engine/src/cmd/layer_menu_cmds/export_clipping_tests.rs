//! #14813: Layer › Export As and Quick Export as PNG of a clipping base include the layers
//! clipped to it, at the base's bounds; a clipped layer exports its whole clipping group.

use super::*;
use photocraft_color::SampleType;
use photocraft_geom::Size;

const GREEN: [f32; 4] = [0.0, 160.0 / 255.0, 0.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

fn square(name: &str, fmt: photocraft_color::PixelFormat, rect: Rect, color: [f32; 4]) -> Layer {
    let mut l = Layer::raster(name, fmt);
    l.surface_mut().unwrap().fill_rect(rect, &photocraft_raster::from_rgba(&fmt, color));
    l
}

/// 140 x 140 with no Background: `Base` green 60 x 60 at (20,20), in Screen; `Clip` yellow
/// 60 x 60 at (50,50) clipped to it; `Other` (unclipped) a green square at half opacity.
fn doc() -> Document {
    let mut doc = Document::new("Clipping", Size::new(140, 140), ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let mut base = square("Base", fmt, Rect::new(20, 20, 80, 80), GREEN);
    base.blend = BlendMode::Screen;
    let mut clip = square("Clip", fmt, Rect::new(50, 50, 110, 110), YELLOW);
    clip.clipped = true;
    let mut other = square("Other", fmt, Rect::new(100, 0, 120, 20), GREEN);
    other.opacity = 0.5;
    doc.layers = vec![base, clip, other];
    doc
}

fn id(doc: &Document, name: &str) -> LayerId {
    doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == name).unwrap().id
}

fn pixel(doc: &Document, x: usize, y: usize) -> [u8; 4] {
    let w = doc.size.width as usize;
    photocraft_compose::flatten(doc).into_px()[y * w + x].map(|v| (v * 255.0).round() as u8)
}

#[test]
fn a_clipping_base_exports_with_its_clipped_layers_at_its_bounds() {
    let doc = doc();
    let out = layer_document(&doc, id(&doc, "Base")).unwrap();
    assert_eq!((out.size.width, out.size.height), (60, 60), "the base's bounds");
    assert_eq!(pixel(&out, 0, 0), [0, 160, 0, 255]);
    assert_eq!(pixel(&out, 30, 30), [255, 255, 0, 255], "the clipped layer is included");
    assert_eq!(pixel(&out, 59, 59), [255, 255, 0, 255]);
    assert_eq!(pixel(&out, 0, 59), [0, 160, 0, 255], "clipped to the base's pixels");
}

#[test]
fn a_clipped_layer_exports_the_same_image_as_its_base() {
    let doc = doc();
    let base = layer_document(&doc, id(&doc, "Base")).unwrap();
    let clip = layer_document(&doc, id(&doc, "Clip")).unwrap();
    assert_eq!((clip.size.width, clip.size.height), (60, 60));
    assert_eq!(photocraft_compose::flatten(&clip).into_px(), photocraft_compose::flatten(&base).into_px());
    assert_eq!(clip.name, "Clip", "named after the exported layer");
}

#[test]
fn a_clipping_group_inside_a_group_and_unclipped_layers_export_as_before() {
    let mut nested = doc();
    let layers = std::mem::take(&mut nested.layers);
    nested.layers = vec![Layer::group("Folder", layers)];
    let out = layer_document(&nested, id(&nested, "Clip")).unwrap();
    assert_eq!((out.size.width, out.size.height), (60, 60));
    assert_eq!(pixel(&out, 0, 0), [0, 160, 0, 255], "the group's own clipping stack: the base's green");
    assert_eq!(pixel(&out, 30, 30), [255, 255, 0, 255]);
    let plain = doc();
    let other = layer_document(&plain, id(&plain, "Other")).unwrap();
    assert_eq!((other.size.width, other.size.height), (20, 20));
    let p = pixel(&other, 5, 5);
    assert_eq!((p[1], p[3]), (160, 128), "its own opacity, Normal blend");
}

#[test]
fn quick_export_as_png_gives_the_same_pixels() {
    let dir = std::env::temp_dir().join(format!("pc-clip-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Clip.png").to_string_lossy().into_owned();
    let mut s = Session::new();
    s.add_document(doc(), None);
    let doc = s.active().unwrap().doc.clone();
    s.execute("layer.quickExportAsPng", json!({"layer": id(&doc, "Clip").0, "path": path})).unwrap();
    let written = photocraft_io::import(&path, &std::fs::read(&path).unwrap()).unwrap().document;
    let want = layer_document(&doc, id(&doc, "Base")).unwrap();
    assert_eq!((written.size.width, written.size.height), (60, 60));
    assert_eq!(pixel(&written, 30, 30), pixel(&want, 30, 30));
    assert_eq!(pixel(&written, 0, 0), pixel(&want, 0, 0));
    std::fs::remove_dir_all(&dir).unwrap();
}
