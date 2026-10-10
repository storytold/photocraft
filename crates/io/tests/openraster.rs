use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Group, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_io::{ExportOptions, export, import};
use photocraft_raster::Surface;
fn doc(depth: SampleType) -> Document {
    let mut doc = Document::new("ORA synthetic", Size::new(16, 16), ColorMode::Rgb, depth);
    doc.resolution_dpi = 144.0;
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
    surface.write_region(Rect::new(0, 0, 16, 16), &[0.2, 0.4, 0.6, 1.0].repeat(256));
    let bottom = Layer::new("Bottom & <base>", LayerContent::Raster(surface.clone()));
    let mut top = Layer::new("Hidden \"ink\"", LayerContent::Raster(surface));
    top.opacity = 0.25;
    top.visible = false;
    top.blend = BlendMode::Multiply;
    let mut group = Layer::new("Group", LayerContent::Group(Group { children: vec![top], expanded: true, artboard: None }));
    group.blend = BlendMode::PassThrough;
    doc.layers = vec![bottom, group];
    doc
}
#[test]
fn layered_order_groups_visibility_opacity_depth_and_pixels_roundtrip() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let original = doc(depth);
        let result = export(&original, "ora", &Default::default()).unwrap();
        let opened = import("roundtrip.ora", &result.bytes).unwrap().document;
        assert_eq!(opened.size, original.size);
        assert_eq!(opened.layers[0].name, original.layers[0].name);
        assert_eq!(opened.resolution_dpi, 144.0);
        assert_eq!(opened.depth, if depth == SampleType::F32 { SampleType::U16 } else { depth });
        let LayerContent::Group(group) = &opened.layers[1].content else { panic!("missing group") };
        assert_eq!(opened.layers[1].blend, BlendMode::PassThrough);
        assert_eq!(group.children[0].name, "Hidden \"ink\"");
        assert!(!group.children[0].visible);
        assert_eq!(group.children[0].opacity, 0.25);
        assert_eq!(group.children[0].blend, BlendMode::Multiply);
        let a = photocraft_compose::flatten(&original).to_rgba8();
        let b = photocraft_compose::flatten(&opened).to_rgba8();
        assert_eq!(a.pixels, b.pixels);
        if depth == SampleType::F32 {
            assert!(result.warnings.iter().any(|w| w.contains("32-bit")));
        }
        if let Ok(dir) = std::env::var("PHOTOCRAFT_EXPORT_ORACLE_DIR") {
            std::fs::write(std::path::Path::new(&dir).join(format!("layers-{}.ora", depth.bits())), result.bytes).unwrap();
        }
    }
}
fn archive(stack: &str, layer: Option<&[u8]>) -> Vec<u8> {
    use std::io::{Cursor, Write};
    use zip::{ZipWriter, write::SimpleFileOptions};
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [("mimetype", b"image/openraster".as_slice()), ("stack.xml", stack.as_bytes())] {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    if let Some(bytes) = layer {
        zip.start_file("data/l.png", SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
#[test]
fn external_stack_with_signed_layer_offsets() {
    let image = photocraft_codecs::Image::from_u8(2, 1, photocraft_codecs::ChannelLayout::Rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let bytes = archive("<image w='8' h='8' version='0.0.6'><stack><layer name='offset' src='data/l.png' x='-1' y='2'/></stack></image>", Some(&png));
    let opened = import("offset.ora", &bytes).unwrap().document;
    let px = photocraft_compose::flatten(&opened).to_rgba8();
    assert_eq!(&px.pixels[(2 * 8) * 4..(2 * 8) * 4 + 4], &[0, 255, 0, 255]);
}
#[test]
fn corrupt_xml_zip_paths_sizes_and_depth_fail_without_panicking() {
    for xml in [
        "<image w='4294967295' h='4294967295'><stack/></image>",
        "<image w='1' h='1'><stack><layer src='../escape.png'/></stack></image>",
        "<!DOCTYPE image [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><image w='1' h='1'><stack/></image>",
        "<image w='1' h='1'><stack><layer src='data/l.png' opacity='NaN'/></stack></image>",
    ] {
        let bytes = archive(xml, None);
        assert!(import("bad.ora", &bytes).is_err());
    }
    let xml = format!("<image w='1' h='1'><stack>{}{}</stack></image>", "<stack>".repeat(102), "</stack>".repeat(102));
    assert!(import("deep.ora", &archive(&xml, None)).is_err());
    let bytes = export(&doc(SampleType::U8), "ora", &ExportOptions::default()).unwrap().bytes;
    for n in 0..bytes.len() {
        assert!(std::panic::catch_unwind(|| import("truncated.ora", &bytes[..n])).is_ok());
    }
}
#[test]
fn unsupported_backdrop_features_export_a_warned_visual_composite() {
    let mut original = doc(SampleType::U8);
    original.layers[0].blend = BlendMode::VividLight;
    let result = export(&original, "ora", &Default::default()).unwrap();
    assert!(result.warnings.iter().any(|w| w.contains("flattened composite")));
    let opened = import("fallback.ora", &result.bytes).unwrap().document;
    assert_eq!(opened.layers.len(), 1);
    assert_eq!(photocraft_compose::flatten(&opened).to_rgba8().pixels, photocraft_compose::flatten(&original).to_rgba8().pixels);
}

#[test]
fn invalid_layer_names_fail_before_creating_an_unreadable_file() {
    let mut original = doc(SampleType::U8);
    original.layers[0].name = "bad\0name".into();
    assert!(export(&original, "ora", &Default::default()).is_err());
}
