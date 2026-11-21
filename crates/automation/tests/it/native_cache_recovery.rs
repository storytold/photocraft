//! Native automation opens restore eligible embedded pixels through the ordinary readers.

use photocraft_automation::{AuthorizedWorkspace, Headless, files};
use photocraft_codecs::{ChannelLayout, Format, Image};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Affine, Document, Layer, LayerContent, Size, SmartObject, SmartSource};
use std::path::Path;
use std::sync::Arc;

const PIXELS: [u8; 16] = [19, 44, 133, 255, 207, 104, 61, 255, 92, 77, 90, 255, 11, 208, 183, 255];

fn png(pixels: Vec<u8>) -> Vec<u8> {
    let image = Image::from_u8(2, 2, ChannelLayout::Rgba, pixels).unwrap();
    photocraft_codecs::encode(&image, Format::Png, &Default::default()).unwrap()
}

fn assert_recovered(doc: &Document, warnings: &[String]) {
    assert_eq!(photocraft_compose::render(doc, doc.bounds()).to_rgba8().pixels, PIXELS);
    let LayerContent::Smart(link) = &doc.layers[1].content else { panic!("the linked layer remains editable") };
    assert!(link.cache.is_none(), "an open must not read a linked file to build a cache");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("Linked art") && warnings[0].contains("no saved render cache"), "{warnings:?}");
}

#[test]
fn automation_rebuilds_eligible_caches_on_native_open() {
    let scratch = std::env::temp_dir().join(format!("photocraft-automation-native-open-{}", std::process::id()));
    let root = scratch.join("root");
    std::fs::create_dir_all(&root).unwrap();
    let outside = scratch.join("outside.png");
    std::fs::write(&outside, png([255, 0, 0, 255].repeat(4))).unwrap();
    let mut doc = Document::new("Native", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let embedded = SmartSource::Embedded { file_name: "source.png".into(), bytes: Arc::new(png(PIXELS.to_vec())) };
    let linked = SmartSource::Linked { path: outside.to_string_lossy().into_owned() };
    for (name, source) in [("Embedded art", embedded), ("Linked art", linked)] {
        doc.layers.push(Layer::new(name, LayerContent::Smart(SmartObject::new(source, Affine::IDENTITY, None))));
    }

    let bundle = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let renamed = root.join("renamed.png");
    std::fs::write(&renamed, &bundle).unwrap();
    let directory = root.join("Document.pcraft");
    photocraft_format::PcraftWriter::new().save_dir(&doc, &directory, &Default::default()).unwrap();

    let opened = [
        files::open_bytes("Document.pcraft", &bundle).unwrap(),
        files::open_bytes("renamed.png", &bundle).unwrap(),
        files::open(&renamed).unwrap(),
        files::open(&directory).unwrap(),
    ];

    // A CLI Convert saves the document returned by files::open.
    let output = scratch.join("converted.png");
    files::save(&opened[2].document, &output, None, &Default::default(), None).unwrap();
    let converted = photocraft_codecs::decode(&std::fs::read(&output).unwrap()).unwrap();
    let workspace = AuthorizedWorkspace::new(Some(&root), None).unwrap();
    let mut headless = Headless::with_workspace(workspace);
    let report = headless.open(Path::new("renamed.png")).unwrap();
    let warnings: Vec<String> = serde_json::from_value(report["warnings"].clone()).unwrap();
    let local_file = std::fs::read(&renamed).unwrap();
    let state = headless.session.active().unwrap();
    let (scoped_doc, save_path) = (state.doc.clone(), state.path.clone());
    drop(headless);

    std::fs::remove_dir_all(scratch).unwrap();
    assert_recovered(&scoped_doc, &warnings);
    for result in opened {
        assert_recovered(&result.document, &result.warnings);
    }
    assert_eq!(converted.to_rgba8(), PIXELS);
    assert_eq!(local_file, bundle, "opening leaves the original native file alone");
    assert!(save_path.is_none(), "read-only authority does not acquire a save target");
}

#[test]
fn an_explicit_pcraft_byte_open_still_requires_a_native_bundle() {
    let bytes = png(PIXELS.to_vec());
    assert!(matches!(files::open_bytes("Named.PCRAFT", &bytes), Err(photocraft_automation::AutomationError::Format(_))));
    assert!(files::open_bytes("Named.png", &bytes).unwrap().warnings.is_empty());
}
