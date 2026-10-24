//! Published text input follows the normal native preparation/edit/history path.
#![cfg(feature = "corpus")]

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::text::{CharStyle, Kerning, ParagraphRun, ParagraphStyle, TextDirection, TextRun};
use photocraft_doc::{Document, Layer, LayerContent, TextLayer};
use photocraft_engine::{Session, file_cmds::open_bytes_as, jobs::JobCtx};
use photocraft_geom::{Affine, Rect, Size};
use serde_json::json;

fn counterpart() -> Document {
    let mut doc = Document::new("Native", Size::new(160, 60), ColorMode::Rgb, SampleType::U8);
    doc.resolution_dpi = 300.0;
    doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    let mut background = Layer::raster("Background", doc.pixel_format());
    background.locks.transparency = true;
    background.surface_mut().unwrap().write_region(Rect::new(0, 0, 160, 60), &[1.0; 4].repeat(160 * 60));
    let style = CharStyle {
        font_family: "DejaVu Sans".into(),
        font_style: "Book".into(),
        postscript_name: Some("DejaVuSans".into()),
        size_pt: 24.0 * 72.0 / 300.0,
        kerning: Kerning::Off,
        language: Some("en-US".into()),
        ..Default::default()
    };
    let mut text = TextLayer {
        text: "XCF".into(),
        transform: Affine::translate(10.0, 10.0),
        runs: vec![TextRun { len: 3, style }],
        paragraphs: vec![ParagraphRun { len: 3, style: ParagraphStyle { direction: TextDirection::Ltr, ..Default::default() } }],
        ..Default::default()
    };
    text.sync_summary();
    let mut layer = Layer::raster("XCF", doc.pixel_format());
    layer.content = LayerContent::Text(text);
    doc.layers = vec![background, layer];
    doc
}

fn compare(imported: &Session, native: &Session) {
    let a = &imported.active().unwrap().doc;
    let b = &native.active().unwrap().doc;
    assert_eq!(a.layers.len(), b.layers.len());
    for (a, b) in a.layers.iter().zip(&b.layers) {
        let mut a = a.clone();
        a.id = b.id;
        assert_eq!(&a, b, "ordinary native layer state");
    }
    assert_eq!(photocraft_compose::flatten(a).px, photocraft_compose::flatten(b).px, "ordinary native rendering");
}

#[test]
fn first_open_text_prepares_edits_undoes_and_reopens_without_source() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp/text.xcf");
    let bytes = std::fs::read(fixture).expect("run cargo xtask corpus --gimp");
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let directory = std::env::temp_dir().join(format!("photocraft-native-xcf-text-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("source.xcf");
    let saved = directory.join("native.pcraft");
    std::fs::write(&source, &bytes).unwrap();
    let mut imported = Session::new();
    imported.color.settings.ask_on_missing = false;
    open_bytes_as(&mut imported, source.to_str().unwrap(), &bytes, None, Some(source.to_string_lossy().into_owned())).unwrap();
    let report = imported.active().unwrap().doc.metadata.import_conversion.clone();
    let LayerContent::Text(text) = &imported.active().unwrap().doc.layers[1].content else { panic!("editable text") };
    assert!(text.cache.is_some(), "first successful host open prepared native typography");
    let mut native = Session::new();
    native.color.settings.ask_on_missing = false;
    native.open_imported_document(counterpart(), None, &JobCtx::new()).unwrap();
    compare(&imported, &native);
    for (command, mut params) in [
        ("type.edit", json!({"text":"héλlo\n漢字"})),
        ("type.setStyle", json!({"range":[1,3],"size":10,"fauxBold":true})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
    ] {
        let mut native_params = params.clone();
        if command.starts_with("type.") {
            params["layer"] = json!(imported.active().unwrap().doc.layers[1].id.0);
            native_params["layer"] = json!(native.active().unwrap().doc.layers[1].id.0);
        }
        imported.execute(command, params).unwrap();
        native.execute(command, native_params).unwrap();
        compare(&imported, &native);
        assert_eq!(imported.active().unwrap().doc.metadata.import_conversion, report);
    }
    imported.execute("file.saveACopy", json!({"path":saved})).unwrap();
    std::fs::remove_file(&source).unwrap();
    let mut reopened = Session::new();
    reopened.color.settings.ask_on_missing = false;
    open_bytes_as(&mut reopened, saved.to_str().unwrap(), &std::fs::read(&saved).unwrap(), None, None).unwrap();
    assert!(!reopened.active().unwrap().source_read_only);
    compare(&reopened, &native);
    assert_eq!(reopened.active().unwrap().doc.metadata.import_conversion, report);
    reopened.execute("type.edit", json!({"layer":reopened.active().unwrap().doc.layers[1].id.0,"text":"Native work"})).unwrap();
    reopened.execute("edit.undo", json!({})).unwrap();
    compare(&reopened, &native);
    std::fs::remove_file(saved).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
