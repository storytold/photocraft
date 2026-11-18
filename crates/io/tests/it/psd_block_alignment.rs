//! Every layer-level tagged block of an exported PSD/PSB is a multiple of 4 bytes long.
//!
//! Photoshop pads layer-level blocks to 4 inside the length field; all of its own files do (567
//! fixtures checked), and a regenerated block that stopped at an even but unaligned size left the
//! layer record's extra data unaligned: a Pattern Fill's `PtFl` came out 2 bytes short of the
//! boundary for half of all pattern names.

use craft_psd::PsdFile;
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::text::{CharStyle, TextRun};
use photocraft_doc::{Adjustment, Document, Fill, Layer, LayerContent, Path, Pattern, ShapeLayer, ShapeStroke, Subpath, TextLayer, VectorMask};
use photocraft_geom::{Rect, Size};
use photocraft_io::*;
use photocraft_raster::Surface;

fn pattern(name: &str) -> Pattern {
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(0, 0, 8, 8), &[0.8, 0.8, 0.8, 1.0]);
    s.fill_rect(Rect::new(0, 0, 4, 4), &[0.2, 0.4, 0.9, 1.0]);
    Pattern { id: "059289fb-0000-4000-8000-000000000001".into(), name: name.into(), width: 8, height: 8, surface: s }
}

/// One Pattern Fill layer per name; the name's length moves the descriptor's size by two bytes
/// per character, so consecutive lengths cover every residue modulo 4.
fn doc(depth: SampleType, names: &[&str]) -> Document {
    let mut d = Document::new("p", Size::new(40, 24), ColorMode::Rgb, depth);
    for name in names {
        let p = pattern(name);
        let fill = Fill::Pattern { name: (*name).into(), scale: 1.0, id: p.id.clone(), angle: 0.0, link: true, phase: (0.0, 0.0) };
        d.patterns.push(p);
        d.layers.push(Layer::new(format!("Fill {name}"), LayerContent::Fill(fill)));
    }
    d
}

#[test]
fn layer_blocks_are_four_byte_aligned() {
    let names = ["A", "Ab", "Abc", "Abcd", "Dots", "Checker", "Wood grain"];
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for path in ["x.psd", "x.psb"] {
            let bytes = export(&doc(depth, &names), path, &ExportOptions::default()).expect("export").bytes;
            let file = PsdFile::from_bytes(&bytes).expect("reparse");
            let mut fills = 0;
            for layer in file.layers() {
                for b in &layer.blocks {
                    let key = String::from_utf8_lossy(&b.key).into_owned();
                    assert_eq!(b.data.len() % 4, 0, "{depth:?} {path}: layer {:?} block {key} is {} bytes", layer.name(), b.data.len());
                }
                fills += usize::from(layer.block(b"PtFl").is_some());
            }
            assert_eq!(fills, names.len(), "{depth:?} {path}: every pattern fill keeps its PtFl block");
        }
    }
}

#[test]
fn generated_gradient_map_blocks_are_four_byte_aligned() {
    let cases = [(vec![(0.0, [0.04, 0.1, 0.18]), (1.0, [1.0; 3])], 128), (vec![(0.0, [0.04, 0.1, 0.18]), (0.5, [0.4, 0.2, 0.1]), (1.0, [1.0; 3])], 148)];
    for (stops, expected_length) in cases {
        let mut doc = Document::new("gradient", Size::new(16, 16), ColorMode::Rgb, SampleType::U8);
        doc.layers.push(Layer::new("Gradient Map", LayerContent::Adjustment(Adjustment::GradientMap { stops, reverse: false, dither: false, source: None })));
        let bytes = export(&doc, "gradient.psd", &ExportOptions::default()).expect("export").bytes;
        let file = PsdFile::from_bytes(&bytes).expect("reparse");
        let block = file.layers().iter().find_map(|layer| layer.block(b"grdm")).expect("Gradient Map block");
        assert_eq!(block.data.len(), expected_length, "grdm declared length must include padding");
    }
}

/// A file saved by an older build carries the 2-mod-4 `grdm` Photoshop refuses; saving it again,
/// with the stops unchanged or only Reverse toggled, must write an aligned block (#8070).
#[test]
fn legacy_unaligned_gradient_map_is_repaired_on_resave() {
    let stops = vec![(0.0, [0.04, 0.1, 0.18]), (0.5, [0.4, 0.2, 0.1]), (1.0, [1.0; 3])];
    for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
        let mut doc = Document::new("gradient", Size::new(16, 16), mode, SampleType::U8);
        doc.layers.push(Layer::new(
            "Gradient Map",
            LayerContent::Adjustment(Adjustment::GradientMap { stops: stops.clone(), reverse: false, dither: false, source: None }),
        ));
        let mut file = PsdFile::from_bytes(&export(&doc, "gradient.psd", &ExportOptions::default()).expect("export").bytes).expect("reparse");
        let legacy = file.layers_mut().iter_mut().find_map(|layer| layer.block_mut(b"grdm")).expect("Gradient Map block");
        let unpadded = legacy.data.len() - 2;
        legacy.data.truncate(unpadded);
        let legacy = file.to_bytes().expect("legacy file");
        for reverse in [false, true] {
            let mut reopened = import("gradient.psd", &legacy).expect("import").document;
            let Some(LayerContent::Adjustment(Adjustment::GradientMap { reverse: r, .. })) = reopened.layers.iter_mut().map(|l| &mut l.content).last() else {
                panic!("no Gradient Map layer")
            };
            *r = reverse;
            let expected = reopened.layers.last().map(|l| l.content.clone());
            let bytes = export(&reopened, "gradient.psd", &ExportOptions::default()).expect("re-save").bytes;
            let file = PsdFile::from_bytes(&bytes).expect("reparse");
            let block = file.layers().iter().find_map(|layer| layer.block(b"grdm")).expect("Gradient Map block");
            assert_eq!(block.data.len() % 4, 0, "{mode:?} reverse {reverse}: re-saved grdm is {} bytes", block.data.len());
            let again = import("gradient.psd", &bytes).expect("reopen").document;
            assert_eq!(again.layers.last().map(|l| l.content.clone()), expected, "{mode:?} reverse {reverse}: Gradient Map changed on re-save");
        }
    }
}

/// A closed polygon of `corners` corners. A path block holds 26-byte records, so its size is
/// 2 mod 4 for every other corner count. Whole pixels of a 64-px canvas are exact in the PSD's
/// 8.24 fixed point.
fn polygon(corners: usize) -> Path {
    let points: Vec<(f64, f64)> = (0..corners).map(|i| (8.0 + 6.0 * i as f64, if i % 2 == 0 { 8.0 } else { 40.0 })).collect();
    Path::new(vec![Subpath::polygon(&points)])
}

fn shape(path: Path, stroke: Option<ShapeStroke>) -> Layer {
    let mut sh = ShapeLayer { path, fill: Some(Fill::Solid(Color::rgba(0.0, 0.68, 0.93, 1.0))), stroke, ..ShapeLayer::default() };
    sh.cache = Some(photocraft_vector::render_shape(&sh, PixelFormat::RGBA8, Rect::new(0, 0, 64, 64)));
    Layer::new("Shape", LayerContent::Shape(sh))
}

fn masked(path: Path) -> Layer {
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(0, 0, 64, 64), &[0.2, 0.4, 0.9, 1.0]);
    let mut layer = Layer::new("Masked", LayerContent::Raster(s));
    layer.vector_mask = Some(VectorMask { path, enabled: true, linked: true, density: 1.0, feather: 0.0 });
    layer
}

fn type_layer(doc: &Document, text: &str) -> Layer {
    let style = CharStyle { font_family: "Inter".into(), size_pt: 12.0, ..Default::default() };
    let mut t = TextLayer { text: text.into(), runs: vec![TextRun { len: text.len(), style }], ..Default::default() };
    t.sync_summary();
    photocraft_text::TextEngine::new().render_layer(&mut t, doc.resolution_dpi, doc.pixel_format());
    Layer::new(text, LayerContent::Text(t))
}

/// Shapes with and without a stroke, vector masks and type layers, sized so their path, stroke
/// and type blocks land on every residue modulo 4 before padding.
fn vector_and_type_doc() -> Document {
    let mut d = Document::new("v", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
    for corners in 3..=6 {
        d.layers.push(shape(polygon(corners), None));
        d.layers.push(masked(polygon(corners)));
    }
    for width in [1.0, 5.6] {
        d.layers.push(shape(polygon(4), Some(ShapeStroke { width, ..ShapeStroke::default() })));
    }
    for text in ["A", "Ab", "Abc", "Abcd", "One\nTwo"] {
        let layer = type_layer(&d, text);
        d.layers.push(layer);
    }
    d
}

fn unaligned(bytes: &[u8]) -> Vec<String> {
    let file = PsdFile::from_bytes(bytes).expect("reparse");
    let mut out = Vec::new();
    for layer in file.layers() {
        for b in layer.blocks.iter().filter(|b| b.data.len() % 4 != 0) {
            out.push(format!("layer {:?} block {} is {} bytes", layer.name(), String::from_utf8_lossy(&b.key), b.data.len()));
        }
    }
    out
}

/// The type and vector blocks of an exported file are padded to 4 bytes like Photoshop's: a shape
/// path with an odd number of records (`vmsk`), a stroke (`vstk`) and a type layer's `TySh` came
/// out 2 bytes short of the boundary.
#[test]
fn vector_and_type_blocks_are_four_byte_aligned() {
    let doc = vector_and_type_doc();
    for path in ["x.psd", "x.psb"] {
        let bytes = export(&doc, path, &ExportOptions::default()).expect("export").bytes;
        assert_eq!(unaligned(&bytes), Vec::<String>::new(), "{path}");
        let file = PsdFile::from_bytes(&bytes).expect("reparse");
        let count = |key: &[u8; 4]| file.layers().iter().filter(|l| l.block(key).is_some()).count();
        assert_eq!((count(b"vmsk"), count(b"vstk"), count(b"TySh")), (10, 2, 5), "{path}: the blocks under test are written");
        // The padding changes nothing a reader sees.
        let back = import(path, &bytes).expect("reopen").document;
        for (a, b) in doc.layers.iter().zip(&back.layers) {
            match (&a.content, &b.content) {
                (LayerContent::Shape(x), LayerContent::Shape(y)) => {
                    assert_eq!((&x.path, &x.stroke, &x.fill), (&y.path, &y.stroke, &y.fill), "{path}: {}", a.name)
                }
                (LayerContent::Text(x), LayerContent::Text(y)) => assert_eq!(x.text, y.text, "{path}: {}", a.name),
                (LayerContent::Raster(_), LayerContent::Raster(_)) => {
                    assert_eq!(a.vector_mask.as_ref().map(|m| &m.path), b.vector_mask.as_ref().map(|m| &m.path), "{path}: {}", a.name)
                }
                _ => panic!("{path}: layer {} changed kind", a.name),
            }
        }
    }
}

/// A file an earlier release saved holds these blocks 2 bytes short. Saving it again pads them,
/// and the layers read back the same.
#[test]
fn legacy_unaligned_vector_and_type_blocks_are_repaired_on_resave() {
    let doc = vector_and_type_doc();
    let mut file = PsdFile::from_bytes(&export(&doc, "x.psd", &ExportOptions::default()).expect("export").bytes).expect("reparse");
    let mut cut = 0;
    for layer in file.layers_mut().iter_mut() {
        for key in [b"vmsk", b"vstk", b"TySh"] {
            // Only the padding this export added: two zero bytes after a 2-mod-4 payload.
            if let Some(block) = layer.block_mut(key)
                && block.data.ends_with(&[0, 0])
                && cut_is_padding(key, &block.data)
            {
                let len = block.data.len() - 2;
                block.data.truncate(len);
                cut += 1;
            }
        }
    }
    assert!(cut >= 3, "the fixture has blocks to cut ({cut})");
    let legacy = file.to_bytes().expect("legacy file");
    assert_eq!(unaligned(&legacy).len(), cut, "the legacy file holds the unaligned blocks");
    let reopened = import("x.psd", &legacy).expect("import").document;
    let bytes = export(&reopened, "x.psd", &ExportOptions::default()).expect("re-save").bytes;
    assert_eq!(unaligned(&bytes), Vec::<String>::new());
    let again = import("x.psd", &bytes).expect("reopen").document;
    for (a, b) in reopened.layers.iter().zip(&again.layers) {
        assert_eq!(a.vector_mask.as_ref().map(|m| &m.path), b.vector_mask.as_ref().map(|m| &m.path), "{}", a.name);
        if let (LayerContent::Shape(x), LayerContent::Shape(y)) = (&a.content, &b.content) {
            assert_eq!((&x.path, &x.stroke), (&y.path, &y.stroke), "{}", a.name);
        }
        if let (LayerContent::Text(x), LayerContent::Text(y)) = (&a.content, &b.content) {
            assert_eq!(x.text, y.text, "{}", a.name);
        }
    }
}

/// Whether the last two bytes of a `key` block are padding: a path block's records are 26 bytes
/// after its 8-byte header, and the other two blocks are complete without their tail when
/// writing them again from what they parse to gives the same bytes.
fn cut_is_padding(key: &[u8; 4], data: &[u8]) -> bool {
    let body = &data[..data.len() - 2];
    match key {
        b"vmsk" => (data.len() - 8) % 26 == 2,
        b"vstk" => photocraft_io::vector_map::parse_vstk(body, 72.0)
            .is_some_and(|v| photocraft_io::vector_map::vstk_bytes(v.stroke.as_ref(), v.fill_enabled, 72.0) == body),
        _ => photocraft_text::psd::parse_tysh(body).and_then(|t| photocraft_text::psd::write_tysh(&t)).is_some_and(|again| again == body),
    }
}
