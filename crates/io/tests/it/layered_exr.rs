use std::io::Cursor;

use craft_exr::decode::Limits;
use craft_exr::layer_stack::{ATTRIBUTE_NAME, LayerStack};
use craft_exr::seek::Index;
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_geom::{Rect, Size};
use photocraft_io::{LayeredExrOptions, import, write_layered_exr};
use photocraft_raster::Interrupt;

#[test]
fn layered_exr_reopens_nested_layers_and_preserves_blending() {
    let mut doc = Document::new("Scene", Size::new(300, 31), ColorMode::Rgb, SampleType::F32);
    let mut backdrop = Layer::raster("Light 2", PixelFormat::RGBA32F);
    backdrop.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.6, 0.5, 0.25, 1.0]);
    let mut top = Layer::raster("Light 2", PixelFormat::RGBA32F);
    let painted = Rect::from_xywh(-4, 2, 289, 18);
    top.surface_mut().unwrap().fill_rect(painted, &[0.5, 0.75, 1.25, 0.5]);
    top.blend = BlendMode::Multiply;
    top.opacity = 0.6;
    top.fill_opacity = 0.8;
    let mut hidden = Layer::raster("Hidden", PixelFormat::RGBA32F);
    hidden.visible = false;
    hidden.surface_mut().unwrap().fill_rect(Rect::from_xywh(3, 4, 1, 1), &[0.9, 0.3, 0.8, 1.0]);
    doc.layers.push(backdrop);
    doc.layers.push(Layer::group("Group", vec![top, hidden]));

    let mut file = Cursor::new(Vec::new());
    let warnings = write_layered_exr(&doc, &mut file, &LayeredExrOptions::default(), &Interrupt::NONE).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let bytes = file.into_inner();
    if let Some(path) = std::env::var_os("PHOTOCRAFT_EXR_TEST_FIXTURE") {
        std::fs::write(path, &bytes).unwrap();
    }
    let index = Index::build(&mut Cursor::new(&bytes), &Limits::default(), &|| false).unwrap();
    assert_eq!(index.parts().len(), 4, "Beauty + three raster layers");
    assert_eq!(index.parts()[0].name.as_deref(), Some("Beauty"));
    let manifest = LayerStack::parse(index.parts()[0].text(ATTRIBUTE_NAME).unwrap()).unwrap().unwrap();
    assert_eq!(manifest.layers.len(), 4, "the group is a manifest node");

    let actual = import("scene.exr", &bytes).unwrap().document;
    assert_eq!(actual.size, doc.size);
    assert_eq!(actual.layers.len(), 2, "Beauty is not part of the editable stack");
    assert_eq!(actual.layers[0].name, "Light 2");
    let children = actual.layers[1].children().unwrap();
    let top = &children[0];
    assert_eq!(top.name, "Light 2");
    assert_eq!(top.blend, BlendMode::Multiply);
    assert_eq!(top.opacity, 0.6);
    assert_eq!(top.fill_opacity, 0.8);
    assert!(!children[1].visible);
    let source = top.surface().unwrap().read_region(Rect::from_xywh(-2, 6, 1, 1));
    assert_channels(&source, &[0.5, 0.75, 1.25, 0.5]);

    let before = photocraft_compose::render(&doc, doc.bounds());
    let after = photocraft_compose::render(&actual, actual.bounds());
    assert_channels(after.px.as_flattened(), before.px.as_flattened());
}

#[test]
fn layered_exr_keeps_integer_linear_dodge_at_the_original_depth() {
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA16] {
        let mut doc = Document::new("Additive light", Size::new(1, 1), ColorMode::Rgb, format.sample);
        let mut base = Layer::raster("Base", format);
        base.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.8, 0.6, 0.0, 1.0]);
        let mut light = Layer::raster("Light", format);
        light.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.7, 0.1, 0.213_45, 1.0]);
        light.blend = BlendMode::LinearDodge;
        doc.layers.push(base);
        doc.layers.push(light);
        let before = photocraft_compose::render(&doc, doc.bounds());
        assert_eq!(before.px[0][0], 1.0, "integer Linear Dodge saturates");

        let mut file = Cursor::new(Vec::new());
        write_layered_exr(&doc, &mut file, &LayeredExrOptions::default(), &Interrupt::NONE).unwrap();
        let reopened = import("light.exr", file.get_ref()).unwrap().document;
        assert_eq!(reopened.depth, format.sample);
        for (original, restored) in doc.layers.iter().zip(reopened.layers.iter()) {
            let (original, restored) = (original.surface().unwrap(), restored.surface().unwrap());
            assert_eq!(restored.format(), format);
            assert_eq!(restored.rgba(0, 0), original.rgba(0, 0));
        }
        let after = photocraft_compose::render(&reopened, reopened.bounds());
        assert_channels(after.px.as_flattened(), before.px.as_flattened());
    }
}

#[test]
fn layered_exr_keeps_signed_srgb_hdr_pixels() {
    let mut doc = Document::new("HDR", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    let mut layer = Layer::raster("Signed", PixelFormat::RGBA32F);
    let pixel = [-0.5, -0.025, 1.5, 0.4];
    layer.surface_mut().unwrap().fill_rect(doc.bounds(), &pixel);
    doc.layers.push(layer);

    let mut file = Cursor::new(Vec::new());
    write_layered_exr(&doc, &mut file, &LayeredExrOptions::default(), &Interrupt::NONE).unwrap();
    let reopened = import("signed.exr", file.get_ref()).unwrap().document;
    assert_eq!(reopened.depth, SampleType::F32);
    assert_channels(&reopened.layers[0].surface().unwrap().rgba(0, 0), &pixel);
}

fn assert_channels(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-4, "{a} differs from {e}");
    }
}

/// Run in a fresh process with PHOTOCRAFT_EXR_PROBE_SIDE set to 4096 or 16384.
#[test]
#[ignore = "manual disk export memory probe"]
fn layered_exr_memory_probe() {
    let side = std::env::var("PHOTOCRAFT_EXR_PROBE_SIDE").unwrap().parse().unwrap();
    let path = std::env::var_os("PHOTOCRAFT_EXR_PROBE_PATH").unwrap();
    let mut doc = Document::new("Memory probe", Size::new(side, side), ColorMode::Rgb, SampleType::F32);
    doc.icc_profile = Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes());
    let mut base = Layer::raster("Base", PixelFormat::RGBA32F);
    // A full-area solid has shared tile storage so the process peak reflects export buffers.
    base.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.2, 0.3, 0.45, 1.0]);
    let mut detail = Layer::raster("Detail", PixelFormat::RGBA32F);
    detail.surface_mut().unwrap().fill_rect(Rect::from_xywh(8, 24, 511, 256), &[0.4, 0.75, 1.8, 0.5]);
    doc.layers.push(base);
    doc.layers.push(detail);
    let start = std::time::Instant::now();
    let mut file = std::fs::OpenOptions::new().create_new(true).write(true).read(true).open(path).unwrap();
    write_layered_exr(&doc, &mut file, &LayeredExrOptions::default(), &Interrupt::NONE).unwrap();
    file.sync_all().unwrap();
    println!("{side}px: {} bytes in {:.3}s", file.metadata().unwrap().len(), start.elapsed().as_secs_f64());
}
