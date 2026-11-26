//! Original bounded XCF wire fixture; exercise native editing without an engine dependency.
use photocraft_doc::text::TextDirection;
use photocraft_doc::{Document, LayerContent, TextLayer};
use photocraft_text::TextEngine;

const INITIAL: &str = "ABC 123 !";
const EDITED: &str = "אבג 123 !";

fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_be_bytes());
}
fn string(bytes: &mut Vec<u8>, value: &str) {
    word(bytes, value.len() as u32 + 1);
    bytes.extend(value.as_bytes());
    bytes.push(0);
}
fn property(bytes: &mut Vec<u8>, kind: u32, value: &[u8]) {
    word(bytes, kind);
    word(bytes, value.len() as u32);
    bytes.extend(value);
}
fn slot(bytes: &mut Vec<u8>) -> usize {
    let offset = bytes.len();
    bytes.extend(0u64.to_be_bytes());
    offset
}
fn patch(bytes: &mut [u8], offset: usize, target: usize) {
    bytes[offset..offset + 8].copy_from_slice(&(target as u64).to_be_bytes());
}
fn source(direction: &str) -> Vec<u8> {
    // A 1x1 saved type cache in a roomy fixed text box. No font file or producer asset.
    let config =
        format!("(text \"{INITIAL}\") (font \"Inter\") (font-size 20) (font-size-unit points) (box-mode fixed) (box-width 220) (box-height 70) {direction}");
    let mut parasite = Vec::new();
    string(&mut parasite, "gimp-text-layer");
    word(&mut parasite, 1);
    word(&mut parasite, config.len() as u32);
    parasite.extend(config.as_bytes());
    let mut bytes = b"gimp xcf v011\0".to_vec();
    for value in [256, 96, 0, 150, 0, 0] {
        // Canvas, RGB8 precision, property terminator.
        word(&mut bytes, value);
    }
    let layer = slot(&mut bytes);
    slot(&mut bytes); // Layer-list terminator.
    slot(&mut bytes); // Channel-list terminator; v11 has no standalone path table.
    let at = bytes.len();
    patch(&mut bytes, layer, at);
    for value in [1, 1, 1] {
        word(&mut bytes, value);
    }
    string(&mut bytes, "Saved directional name");
    property(&mut bytes, 26, &0u32.to_be_bytes());
    property(&mut bytes, 21, &parasite);
    for value in [0, 0] {
        word(&mut bytes, value);
    }
    let hierarchy = slot(&mut bytes);
    slot(&mut bytes); // No mask.
    let at = bytes.len();
    patch(&mut bytes, hierarchy, at);
    for value in [1, 1, 4] {
        word(&mut bytes, value);
    }
    let level = slot(&mut bytes);
    slot(&mut bytes);
    let at = bytes.len();
    patch(&mut bytes, level, at);
    word(&mut bytes, 1);
    word(&mut bytes, 1);
    let tile = slot(&mut bytes);
    slot(&mut bytes);
    let at = bytes.len();
    patch(&mut bytes, tile, at);
    bytes.extend([17, 31, 57, 255]);
    bytes
}

fn text(document: &Document) -> &TextLayer {
    let LayerContent::Text(text) = &document.layers[0].content else { panic!("import must preserve editable type") };
    text
}
fn reopen(document: &Document) -> Document {
    let saved = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
    photocraft_format::load_from_bytes(&saved).unwrap()
}

/// UAX9's trailing neutral punctuation follows the paragraph's base direction.
/// Compare visual cluster positions, rather than a chosen pixel coordinate or font metric.
fn punctuation_is_left(engine: &mut TextEngine, layer: &TextLayer, dpi: f32) -> bool {
    let layout = engine.layout(layer, dpi);
    assert!(!layout.glyphs.is_empty(), "actual native glyph layout must run");
    let punctuation = layer.text.rfind('!').unwrap();
    let first = layout.clusters.iter().find(|cluster| cluster.range.contains(&0)).unwrap();
    let last = layout.clusters.iter().find(|cluster| cluster.range.contains(&punctuation)).unwrap();
    let first_center = first.x + first.advance * 0.5;
    let punctuation_center = last.x + last.advance * 0.5;
    assert!((first_center - punctuation_center).abs() > 1., "separate visual clusters");
    punctuation_center < first_center
}

#[test]
fn automatic_and_explicit_directions_survive_reopen_and_real_native_text_changes() {
    for (fields, expected, initial_left, edited_left) in [
        ("", TextDirection::Auto, false, true),
        ("(base-direction rtl) (base-dir-auto yes)", TextDirection::Auto, false, true),
        ("(base-direction ltr)", TextDirection::Ltr, false, false),
        ("(base-direction rtl)", TextDirection::Rtl, true, true),
    ] {
        let imported = photocraft_io::import("direction.xcf", &source(fields)).unwrap();
        assert!(imported.source_read_only && !imported.preview_only);
        assert_eq!(imported.document.layers[0].name, "Saved directional name");
        let original = text(&imported.document);
        assert_eq!(original.text, INITIAL);
        assert_eq!(original.paragraphs[0].style.direction, expected);
        assert_eq!(original.cache.as_ref().unwrap().pixel(0, 0), [17. / 255., 31. / 255., 57. / 255., 1.]);
        let mut document = reopen(&imported.document);
        assert_eq!(document.layers, imported.document.layers);
        assert_eq!(document.metadata.import_conversion, imported.document.metadata.import_conversion);
        let dpi = document.resolution_dpi;
        let format = document.pixel_format();
        let mut engine = TextEngine::new();
        assert_eq!(punctuation_is_left(&mut engine, text(&document), dpi), initial_left, "before edit: {fields}");
        let LayerContent::Text(edited) = &mut document.layers[0].content else { panic!("native reopened type") };
        edited.text = EDITED.into();
        edited.runs[0].len = edited.text.len();
        edited.paragraphs[0].len = edited.text.len();
        engine.render_layer(edited, dpi, format);
        assert!(!edited.cache.as_ref().unwrap().content_bounds().is_empty(), "real native text pixels replace the saved cache");
        assert_eq!(edited.paragraphs[0].style.direction, expected, "editing must retain the requested policy");
        assert_eq!(punctuation_is_left(&mut engine, edited, dpi), edited_left, "after edit: {fields}");
        let reopened = reopen(&document);
        assert_eq!(text(&reopened).text, EDITED);
        assert_eq!(text(&reopened).paragraphs[0].style.direction, expected);
        assert_eq!(punctuation_is_left(&mut engine, text(&reopened), dpi), edited_left, "edited reopen: {fields}");
        assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
    }
}
